//! A chat's orchestration plan: phases of delegated work the main agent declares
//! and drives with the `workspace_orchestrate` tool.
//!
//! The plan lives in the PARENT session's `extension_data` under
//! [`PLAN_STATE`]/[`PLAN_VERSION`] and is written only by the parent's own tool
//! calls (declaring phases, starting and completing them, writing hand-offs, and
//! a spawn into a phase recording its child). A child's status is never copied
//! into the plan: [`snapshot`] derives each phase's counts from the subagent
//! roster when the plan is read, so the plan cannot drift from what the children
//! actually did.
//!
//! "Meetings" between phases are hand-offs: when a phase completes, its results
//! (written by the parent, or composed from the children's result lines) are
//! injected into the instructions of every child later spawned into a phase that
//! depends on it.
//!
//! Every write goes through [`SessionManager::update_extension_state`], the
//! per-key transaction, never through a whole-blob `extension_data` rewrite.
//! That only protects the plan from writers that are transactions too. The two
//! the model can drive in the same tool batch as a plan action or a spawn, the
//! todo extension (`TodoClient::with_state`) and the goal (`persist_goal`,
//! `clear_persisted_goal`), were moved onto the transactional writes for this
//! reason: as `get_session` then `update().extension_data()` they put back the
//! blob they had read, erasing a plan or a [`record_spawn`] committed in
//! between. A writer that still has that shape can still do so, so check a new
//! writer of the parent's `extension_data` before assuming it is safe.
//!
//! A spawn into a phase and `complete` of that phase are ordered by
//! [`begin_phase_spawn`]: the spawn holds its guard from before the phase check
//! until its child is visible to the barrier, records the child before the child
//! runs, and is refused (its child session deleted) when the phase closed in
//! between. `complete` refuses while a guard is held, or when one was taken while
//! it ran.
//!
//! ⚠ **A child can be more private than its parent.** A spawn never changes the
//! tier, but a child that touches a private model or data source mid-run is
//! ratcheted private while its parent stays public. Nothing such a child wrote
//! may reach the parent through the plan: [`phase_agent_states`] marks it
//! `withheld` (its result line and its session name are dropped), so the
//! automatic hand-off, the barrier's refusal and everything rendered from the
//! stored hand-off carry only what the parent's own tier may hold.
//!
//! # Decision gates
//!
//! A phase may carry a [`PhaseGate`]: 1 to 8 yes/no questions a decision model
//! answers before `complete` (status completed) is allowed to close the phase.
//! The model reads one state built from the phase's goal, its hand-off and its
//! children's result lines ([`gate_state_text`]), the same screened lines the
//! automatic hand-off uses, so a withheld child's text never reaches it. Every
//! question must reach P(yes) at or above the gate's threshold. A failed gate
//! keeps the phase in progress and hands the failing questions back to the
//! model ([`apply_gate_verdict`]); after `max_attempts` failures the phase is
//! blocked. An error never passes and does not use an attempt, and two errors
//! in a row block the phase. Each evaluation by a decision model is one row of
//! the decision log (origin `gate`, origin id the phase id), never a
//! transcript row.
//!
//! ## A gate needs only a language model
//!
//! A gate that names no model is judged by the decision model the chat would
//! use, and when the chat has none it may use, or that one cannot be reached,
//! by the chat's own language model ([`judge_gate`]): one short completion
//! that answers each question yes or no with a P(yes), read strictly, and
//! then treated exactly as a decision model's verdict. Such a gate is never
//! refused when it is declared ([`gate_judge_for`]). The gate's record keeps
//! who gave its last verdict and why ([`GateJudge`]). A gate that names a
//! decision model asks only that model, and is refused where the chat may not
//! use it.
//!
//! ## A gate binds the agent
//!
//! A stored gate binds the agent whoever set it ([`GateOwner`]: the agent, the
//! person or a workflow). The agent may add a gate to an ungated phase (with a
//! threshold of at least [`MIN_AGENT_GATE_THRESHOLD`]) and may repeat a stored
//! gate unchanged or leave it out of a revision. It may not change or remove
//! one, drop a gated phase, change a gated open phase's title, goal or kind,
//! or skip a gated phase ([`apply_complete`], [`apply_agent_complete`]).
//!
//! An open gated phase HOLDS every phase listed after it ([`hold_of`]): a held
//! phase cannot start, take a subagent or complete with status completed until
//! the gate passes or the person passes or skips the gated phase. Position, not
//! `depends_on`, decides, so leaving `depends_on` empty cannot dodge a gate.
//! Work outside the plan (a spawn with no phase, the agent's own tools) is not
//! held; monitors supervise that.
//!
//! Only the person changes, removes, passes, skips or retries a gate
//! ([`person_set_gate`], [`person_remove_gate`], [`person_gate_decision`]). A
//! pass or skip the barrier cannot apply yet (a subagent of the phase is still
//! working) STANDS in the gate's record, and the agent's next `complete` with
//! the matching status applies it without asking the gate.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::agents::subagent_roster::{SubagentRow, SubagentRunStatus};
use crate::decision::settings::{MonitorSpec, MAX_AGENT_MONITORS};
use crate::decision::types::validate_questions;
use crate::decision::{
    DecisionAdapter, DecisionContext, DecisionKind, DecisionModelRef, DecisionRequest,
    DecisionService, DecisionSession, ModelSource, Question, Threshold,
};
use crate::privacy::SessionClassification;
use crate::session::SessionManager;

pub const PLAN_STATE: &str = "orchestration_plan";
pub const PLAN_VERSION: &str = "v1";

/// At most this many phases in one plan.
pub const MAX_PHASES: usize = 12;
/// At most this many subagents recorded across a whole plan. Matches the
/// in-flight spawn cap, so a plan can never describe more children than the
/// process could ever have run at once.
pub const MAX_PLAN_AGENTS: usize = 64;
/// Phase ids are slugs: `[a-z0-9-]{1,40}`.
pub const MAX_PHASE_ID_CHARS: usize = 40;
pub const MAX_TITLE_CHARS: usize = 80;
pub const MAX_GOAL_CHARS: usize = 500;
/// A hand-off summary, as the parent writes it or as it is auto-composed.
pub const MAX_HANDOFF_CHARS: usize = 4_000;
/// A role label recorded with a spawn. Longer labels are shortened, not refused,
/// because by the time [`record_spawn`] runs the child already exists.
pub const MAX_ROLE_CHARS: usize = 40;
/// The whole "Results from earlier phases" block a child receives.
pub const HANDOFF_CONTEXT_MAX_CHARS: usize = 12_000;
/// The hand-off as a snapshot carries it, for display.
pub const SNAPSHOT_HANDOFF_MAX_CHARS: usize = 600;
/// An active child with no recorded activity for this long counts as quiet.
pub const QUIET_AFTER_SECONDS: u64 = 180;
/// Questions on one phase gate.
pub const MAX_GATE_QUESTIONS: usize = 8;
/// Attempts a gate may be given.
pub const MAX_GATE_ATTEMPTS: u32 = 5;
/// The threshold a gate gets when the plan names none.
pub const DEFAULT_GATE_THRESHOLD: f64 = 0.8;
/// The lowest threshold a gate the AGENT declares may have: below it, a gate
/// passes a phase the decision model thinks more likely unfinished than done.
/// A person or a workflow may set any value from 0 to 1.
pub const MIN_AGENT_GATE_THRESHOLD: f64 = 0.5;
/// What the revision reply adds when it changes the monitors of a phase that
/// already has subagents: a phase's monitors reach only later spawns.
pub const PHASE_MONITORS_NOT_RETROACTIVE: &str = "Subagents already in this phase keep the \
     monitors they started with; use workspace_monitor to change theirs.";
/// Evaluations in a row that end in an error before the phase is blocked.
pub const GATE_ERRORS_TO_BLOCK: u32 = 2;
/// The state a gate's decision model reads, at most. Sized like the monitor
/// window's default; [`fit_gate_state`] narrows it further to the gate
/// model's own window.
pub const GATE_STATE_MAX_CHARS: usize = 6_000;
/// The fewest characters a gate's state is narrowed to. A model too small
/// for even this is refused by the size check before sending, which names its
/// window, rather than asked about nothing.
pub const GATE_STATE_MIN_CHARS: usize = 400;
/// Rounds of narrowing [`fit_state_to_window`] makes before it sends what it
/// has to the size check.
const GATE_FIT_ROUNDS: usize = 6;

/// What a phase is for. Drives the glyph a surface draws; carries no behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PhaseKind {
    Plan,
    Research,
    Execution,
    Implementation,
    Verification,
    Testing,
    Auditing,
    Review,
    /// Agents read earlier phases' results together and reconcile them.
    Meeting,
    Synthesis,
    Other,
}

impl PhaseKind {
    /// Parse the model's word for a kind. Common verb forms are accepted
    /// ("implement", "verify", "test"); anything unrecognised is `Other`, since a
    /// kind only picks a glyph and refusing a plan over one would cost a round
    /// trip for nothing.
    pub fn parse_lenient(raw: &str) -> Self {
        match raw
            .trim()
            .to_ascii_lowercase()
            .replace([' ', '_'], "-")
            .as_str()
        {
            "plan" | "planning" => Self::Plan,
            "research" | "explore" | "exploration" | "investigation" | "investigate" => {
                Self::Research
            }
            "execution" | "execute" | "run" => Self::Execution,
            "implementation" | "implement" | "build" => Self::Implementation,
            "verification" | "verify" => Self::Verification,
            "testing" | "test" | "tests" => Self::Testing,
            "auditing" | "audit" => Self::Auditing,
            "review" | "reviewing" => Self::Review,
            "meeting" | "meet" | "reconcile" => Self::Meeting,
            "synthesis" | "synthesize" | "synthesise" | "summary" => Self::Synthesis,
            _ => Self::Other,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Research => "research",
            Self::Execution => "execution",
            Self::Implementation => "implementation",
            Self::Verification => "verification",
            Self::Testing => "testing",
            Self::Auditing => "auditing",
            Self::Review => "review",
            Self::Meeting => "meeting",
            Self::Synthesis => "synthesis",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PhaseStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
    Skipped,
}

impl PhaseStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Blocked => "blocked",
            Self::Skipped => "skipped",
        }
    }

    /// Closed to new work: nothing can be spawned into it and it cannot be
    /// started or completed again.
    pub fn is_closed(self) -> bool {
        matches!(self, Self::Completed | Self::Skipped)
    }
}

/// Per-phase tallies, derived from the roster at read time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PhaseCounts {
    pub total: u32,
    /// Completed.
    pub done: u32,
    /// Queued, running or cancelling.
    pub running: u32,
    pub blocked: u32,
    /// Failed, incomplete or interrupted.
    pub failed: u32,
    /// Active but with no recorded activity for a while (a heuristic, labelled
    /// as "quiet", never as "stalled" or "dead").
    pub quiet: u32,
}

/// A decision gate on a phase (decision models, wave 2): before the phase can
/// complete, a decision model (or, for a gate that names none in a chat with
/// no decision model it may use, the chat's own model) reads its hand-off and
/// its children's result lines and every question must reach P(yes) >=
/// `threshold`. A failed gate keeps
/// the phase in progress and hands the failing questions back to the model; after
/// `max_attempts` failures the phase is blocked. Errors never pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PhaseGate {
    /// Question name -> question (1..=8, yes/no questions).
    #[schema(value_type = Object)]
    pub questions: indexmap::IndexMap<String, crate::decision::Question>,
    pub threshold: f64,
    pub model: Option<crate::decision::DecisionModelRef>,
    #[serde(default = "default_gate_attempts")]
    pub max_attempts: u32,
    #[serde(default)]
    pub allow_uncalibrated: bool,
    /// Who set the gate. A gate stored before owners existed reads as the
    /// agent's. Whoever set it, a stored gate binds the agent.
    #[serde(default)]
    pub owner: GateOwner,
}

fn default_gate_attempts() -> u32 {
    3
}

/// Who set a phase's gate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GateOwner {
    /// The orchestrating agent, with `workspace_orchestrate`.
    #[default]
    Agent,
    /// The person, in the app or with `biorouter session gates`.
    Person,
    /// The workflow the chat was started from.
    Workflow,
}

impl GateOwner {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Person => "person",
            Self::Workflow => "workflow",
        }
    }

    /// "set by the agent", "set by the person", "set by the workflow".
    pub fn set_by_words(self) -> &'static str {
        match self {
            Self::Agent => "set by the agent",
            Self::Person => "set by the person",
            Self::Workflow => "set by the workflow",
        }
    }
}

/// `, set by the person`, `, set by the workflow`, or nothing for the
/// agent's own gate: the phrase the agent's refusals put after "a gate".
fn owner_phrase(owner: GateOwner) -> &'static str {
    match owner {
        GateOwner::Agent => "",
        GateOwner::Person => ", set by the person",
        GateOwner::Workflow => ", set by the workflow",
    }
}

/// A person's decision on a gate that stands (or closed the phase).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PersonGateAction {
    /// Complete the phase without its gate.
    Pass,
    /// Skip the phase without its gate.
    Skip,
}

impl PersonGateAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Skip => "skip",
        }
    }
}

/// What a person may decide about a gate ([`person_gate_decision`]): pass or
/// skip the phase without asking it, or give it its attempts again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PersonGateChoice {
    Pass,
    Skip,
    Retry,
}

/// Which kind of model gave a gate's verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GateJudgeKind {
    /// A decision model, through the decision service.
    DecisionModel,
    /// The chat's own language model. Only a gate that names no model is
    /// judged this way, when the chat has no decision model it may use or the
    /// one it resolved to could not be reached.
    ChatModel,
}

/// The model that gave a gate's last verdict, as every surface names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GateJudge {
    pub kind: GateJudgeKind,
    /// The model as a person reads it: a decision model's display name, or
    /// the chat model's own name. Never a provider id.
    pub model: String,
    /// Why the chat's own model judged the gate, in one plain sentence
    /// ("This chat has no decision model it may use, so gpt-5.5, the chat's
    /// model, judged the gate."). None when a decision model answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl GateJudge {
    /// "gpt-5.5, the chat's model" or "joey-1, a decision model".
    pub fn words(&self) -> String {
        match self.kind {
            GateJudgeKind::ChatModel => format!("{}, the chat's model", self.model),
            GateJudgeKind::DecisionModel => format!("{}, a decision model", self.model),
        }
    }
}

/// A phase gate as surfaces draw it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PhaseGateSnapshot {
    pub threshold: f64,
    pub question_count: u32,
    pub attempts: u32,
    pub max_attempts: u32,
    /// The last evaluation's verdict, when there was one.
    pub last_passed: Option<bool>,
    /// The lowest P(yes) across the gate's questions at the last evaluation.
    pub last_min_p: Option<f64>,
    /// The open phase is blocked by its gate: every attempt failed, or it
    /// could not be evaluated too many times in a row. Never set once the
    /// phase is completed or skipped.
    pub blocked: bool,
    /// Who set the gate.
    #[serde(default)]
    pub owner: GateOwner,
    /// Question name -> the question as asked.
    #[serde(default)]
    #[schema(value_type = HashMap<String, String>)]
    pub questions: IndexMap<String, String>,
    /// The decision model the gate names, as `provider/model`; none means
    /// the chat's default decision model, else the configured one, else the
    /// chat's own language model.
    #[serde(default)]
    pub model: Option<String>,
    /// The model that gave the last verdict (passed or not), when there was
    /// one: a decision model, or the chat's own model and why.
    #[serde(default)]
    pub last_judge: Option<GateJudge>,
    /// The adapter the gate asks `model` through, when it names one; none
    /// means the model's own. An editor that writes the gate back sends it
    /// with the model, because the write replaces the whole gate.
    #[serde(default)]
    pub model_adapter: Option<DecisionAdapter>,
    #[serde(default)]
    pub allow_uncalibrated: bool,
    /// Why the last evaluation decided nothing, when it did not.
    #[serde(default)]
    pub last_error: Option<String>,
    /// The person's pass or skip: one that stands until the agent completes
    /// the phase, or the one that closed it.
    #[serde(default)]
    pub person_decision: Option<PersonGateAction>,
    /// How many times the person gave the gate its attempts again.
    #[serde(default)]
    pub retries_granted: u32,
    /// The open phase is stuck and waits for the person's attention: its gate
    /// blocked it, or it was marked blocked. Only a gate that failed every
    /// attempt leaves the agent no way on; after an error block or its own
    /// block the agent may still ask the gate. Never set once the phase is
    /// completed or skipped.
    #[serde(default)]
    pub needs_person: bool,
    /// Ids of the phases this gate holds now.
    #[serde(default)]
    pub holds: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PhaseSnapshot {
    pub id: String,
    pub title: String,
    pub goal: Option<String>,
    pub kind: PhaseKind,
    pub status: PhaseStatus,
    pub depends_on: Vec<String>,
    /// Child session ids spawned into this phase, in spawn order.
    pub agent_ids: Vec<String>,
    pub counts: PhaseCounts,
    /// The hand-off summary, truncated for display.
    pub handoff: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    /// The phase's decision gate, when it has one.
    #[serde(default)]
    pub gate: Option<PhaseGateSnapshot>,
    /// The id of the earlier gated phase this open phase waits for, when one
    /// holds it.
    #[serde(default)]
    pub held_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationSnapshot {
    pub title: String,
    pub goal: Option<String>,
    pub revision: u64,
    pub phases: Vec<PhaseSnapshot>,
}

// ---------------------------------------------------------------------------
// The stored model. Its fields are private to the crate: every reader outside
// it goes through `snapshot`, which joins it with the roster, and every writer
// through the transitions below. The types are public so a caller outside the
// crate can hold a plan and pass it to them.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrchestrationPlan {
    pub(crate) title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) goal: Option<String>,
    /// Bumped by every write, so a surface can tell a changed plan from a
    /// re-read of the same one.
    #[serde(default)]
    pub(crate) revision: u64,
    #[serde(default)]
    pub(crate) phases: Vec<PlanPhase>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanPhase {
    pub(crate) id: String,
    pub(crate) title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) goal: Option<String>,
    pub(crate) kind: PhaseKind,
    pub(crate) status: PhaseStatus,
    #[serde(default)]
    pub(crate) depends_on: Vec<String>,
    #[serde(default)]
    pub(crate) agents: Vec<PlanAgent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) handoff: Option<PhaseHandoff>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) finished_at: Option<DateTime<Utc>>,
    /// The phase's decision gate, as the plan declared it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) gate: Option<PhaseGate>,
    /// What the gate's evaluations so far came to. Reset when the gate changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) gate_state: Option<GateState>,
    /// Monitors every subagent spawned into this phase from now on starts
    /// with, as the plan declared them (at most [`MAX_AGENT_MONITORS`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) monitors: Vec<MonitorSpec>,
}

/// A person's pass or skip of a gate, and when they gave it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct PersonGateDecision {
    pub(crate) decision: PersonGateAction,
    pub(crate) at: DateTime<Utc>,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

/// A gate's record of its evaluations.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct GateState {
    /// Evaluations that reached a verdict (passed or failed). Errors do not count.
    #[serde(default)]
    pub(crate) attempts: u32,
    /// Evaluations in a row that ended in an error.
    #[serde(default)]
    pub(crate) consecutive_errors: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_passed: Option<bool>,
    /// The lowest P(yes) across the questions at the last verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_min_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_error: Option<String>,
    /// The model that gave the last verdict, kept with `last_passed`. An
    /// error leaves it as it was, as it leaves the verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_judge: Option<GateJudge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_at: Option<DateTime<Utc>>,
    /// A person's pass or skip that stands until the agent completes the
    /// phase with the matching status (a subagent of the phase was still
    /// working when they gave it). Cleared when the person retries the gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) person: Option<PersonGateDecision>,
    /// How many times the person gave the gate its attempts again.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub(crate) retries_granted: u32,
    /// Set when a person's pass or skip closed the phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) closed_by_person: Option<PersonGateAction>,
}

impl GateState {
    /// The person's decision that stands, if any.
    fn standing(&self) -> Option<PersonGateAction> {
        self.person.map(|person| person.decision)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PlanAgent {
    pub(crate) child_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) role: Option<String>,
    pub(crate) spawned_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PhaseHandoff {
    pub(crate) summary: String,
    pub(crate) source: HandoffSource,
    pub(crate) written_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HandoffSource {
    /// Written by the parent (`handoff`, or `complete` with a `summary`).
    Parent,
    /// Composed from the phase's children's result lines when the parent
    /// completed the phase without writing one.
    Auto,
}

impl OrchestrationPlan {
    fn phase(&self, id: &str) -> Option<&PlanPhase> {
        self.phases.iter().find(|phase| phase.id == id)
    }

    fn phase_mut(&mut self, id: &str) -> Option<&mut PlanPhase> {
        self.phases.iter_mut().find(|phase| phase.id == id)
    }

    fn agent_count(&self) -> usize {
        self.phases.iter().map(|phase| phase.agents.len()).sum()
    }

    fn phase_ids(&self) -> String {
        join_ids(self.phases.iter().map(|phase| phase.id.as_str()))
    }

    /// Phases the agent can work on now: open, and not held by an earlier
    /// gate.
    fn open_phase_ids(&self) -> String {
        join_ids(
            self.phases
                .iter()
                .enumerate()
                .filter(|(index, phase)| {
                    !phase.status.is_closed() && self.holder_index(*index).is_none()
                })
                .map(|(_, phase)| phase.id.as_str()),
        )
    }

    fn unknown_phase(&self, id: &str) -> String {
        format!(
            "Phase \"{id}\" is not in this chat's plan. Its phases are: {}.",
            self.phase_ids()
        )
    }

    fn index_of(&self, id: &str) -> Option<usize> {
        self.phases.iter().position(|phase| phase.id == id)
    }

    /// The index of the first phase before `index` whose gate is open (the
    /// phase is neither completed nor skipped): the gate that holds the phase
    /// at `index`.
    fn holder_index(&self, index: usize) -> Option<usize> {
        self.phases
            .iter()
            .take(index)
            .position(|phase| phase.gate.is_some() && !phase.status.is_closed())
    }

    /// The sentence a held phase's start, spawn or completion is refused
    /// with, or `None` when no earlier gate holds `phase_id`.
    fn held_refusal(&self, phase_id: &str) -> Option<String> {
        let holder = hold_of(self, phase_id)?;
        Some(format!(
            "Phase \"{phase_id}\" waits for phase \"{holder}\"'s gate: it cannot start, take a \
             subagent or complete until that gate passes or the person passes or skips \
             \"{holder}\". Work on \"{holder}\" first. Phases you can work on now: {}.",
            self.open_phase_ids()
        ))
    }
}

/// The id of the gated phase that holds `phase_id`: the first phase listed
/// before it whose gate is still open (the phase is neither completed nor
/// skipped). `None` for a phase no gate holds, or one not in the plan.
pub fn hold_of<'a>(plan: &'a OrchestrationPlan, phase_id: &str) -> Option<&'a str> {
    let index = plan.index_of(phase_id.trim())?;
    plan.holder_index(index)
        .map(|holder| plan.phases[holder].id.as_str())
}

fn join_ids<'a>(ids: impl Iterator<Item = &'a str>) -> String {
    let ids: Vec<&str> = ids.collect();
    if ids.is_empty() {
        "none".to_string()
    } else {
        ids.join(", ")
    }
}

const NO_PLAN: &str = "This chat has no orchestration plan. Declare one first with \
     workspace_orchestrate action:\"plan\".";

/// Parse a stored value. A blob this build cannot read is an error, never
/// "no plan": a writer that read it as absent would replace a newer build's
/// plan with an empty one.
fn parse_stored(value: &serde_json::Value) -> anyhow::Result<OrchestrationPlan> {
    serde_json::from_value(value.clone()).map_err(|e| {
        anyhow::anyhow!(
            "this chat's orchestration plan was written in a form this build cannot read \
             ({e}); nothing was changed"
        )
    })
}

/// The stored plan of `parent_session_id`, `Ok(None)` when none was declared.
pub(crate) async fn load_plan(
    session_manager: &SessionManager,
    parent_session_id: &str,
) -> anyhow::Result<Option<OrchestrationPlan>> {
    match session_manager
        .get_extension_state(parent_session_id, PLAN_STATE, PLAN_VERSION)
        .await?
    {
        Some(value) => parse_stored(&value).map(Some),
        None => Ok(None),
    }
}

/// Read-modify-write the plan in ONE per-key transaction. `mutate` sees the
/// stored plan (`None` when none exists) and returns the plan to store plus a
/// value for the caller; an `Err` rolls back and nothing is written. The plan's
/// revision is bumped here, once per successful write, so no action can forget
/// to.
pub(crate) async fn mutate_plan<R, F>(
    session_manager: &SessionManager,
    parent_session_id: &str,
    mutate: F,
) -> anyhow::Result<R>
where
    R: Send,
    F: FnOnce(Option<OrchestrationPlan>) -> Result<(OrchestrationPlan, R), String> + Send,
{
    let mut out: Option<R> = None;
    let stored = session_manager
        .update_extension_state(parent_session_id, PLAN_STATE, PLAN_VERSION, |current| {
            let current = current.map(parse_stored).transpose()?;
            let previous_revision = current.as_ref().map(|plan| plan.revision).unwrap_or(0);
            let (mut next, value) = mutate(current).map_err(anyhow::Error::msg)?;
            next.revision = previous_revision + 1;
            out = Some(value);
            Ok(serde_json::to_value(&next)?)
        })
        .await?;
    match (stored, out) {
        (Some(_), Some(value)) => Ok(value),
        _ => anyhow::bail!("no such chat: {parent_session_id}"),
    }
}

// ---------------------------------------------------------------------------
// Pure transitions. Each takes the stored plan and returns the next one, so the
// rules are testable without a database and the store sees one write per call.
// ---------------------------------------------------------------------------

/// One phase as the model declares it in `action:"plan"`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PlanPhaseInput {
    /// Short slug, unique in the plan: lowercase letters, digits and hyphens,
    /// at most 40 characters (for example "research" or "verify-api").
    pub id: String,
    /// What the phase is, in a few words (at most 80 characters).
    pub title: String,
    /// plan, research, execution, implementation, verification, testing,
    /// auditing, review, meeting, synthesis or other. Any other word is
    /// recorded as other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// What the phase must achieve (at most 500 characters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// Ids of EARLIER phases in this list whose hand-offs this phase's children
    /// receive. Empty means: every earlier phase that has completed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
}

/// One phase as `workspace_orchestrate action:"plan"` takes it: the phase,
/// and optionally a decision gate on its completion and monitors for its
/// subagents.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct OrchestratePhaseInput {
    #[serde(flatten)]
    pub phase: PlanPhaseInput,
    /// Optional decision gate: yes/no questions a decision model answers from
    /// the phase's hand-off and its subagents' results before the phase may
    /// complete. Every question must reach the threshold. Leave it out of a
    /// revision to keep the phase's gate; a stored gate cannot be changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<PhaseGateInput>,
    /// Optional monitors (at most 4) every subagent spawned into this phase
    /// from now on starts with. Left out of a revision, the phase keeps its
    /// monitors; [] removes them for later spawns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitors: Option<Vec<MonitorSpec>>,
}

/// A phase gate as the model declares it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PhaseGateInput {
    /// 1 to 8 yes/no questions, name to question, each answerable from the
    /// phase's results, for example {"tests_pass": "Do the results show every
    /// new test passing?"}. Names are letters, digits, `_` or `-`.
    #[schemars(with = "std::collections::BTreeMap<String, String>")]
    pub questions: IndexMap<String, String>,
    /// The P(yes) every question must reach, from 0.5 to 1. Default 0.8.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    /// Failed evaluations allowed before the phase is blocked, 1 to 5.
    /// Default 3.
    #[serde(
        default,
        alias = "maxAttempts",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_attempts: Option<u32>,
    /// Accept verbalized (uncalibrated) probabilities. Default false: such a
    /// source is refused.
    #[serde(
        default,
        alias = "allowUncalibrated",
        skip_serializing_if = "Option::is_none"
    )]
    pub allow_uncalibrated: Option<bool>,
    /// Optional decision model as provider/model, or by its name when that
    /// names one model decision_models lists (`tev1:0.8b`). Left out, this
    /// chat's default decision model, else the configured default, else the
    /// first model this chat may use; with no decision model, this chat's own
    /// model judges the gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// A gate as the person sets it (`PUT /sessions/{id}/phase-gates/{phase}` and
/// `biorouter session gates`). Validated by [`validate_person_gate`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PhaseGateSpec {
    /// 1 to 8 yes/no questions, name to question. Names are letters, digits,
    /// `_` or `-`.
    #[schema(value_type = HashMap<String, String>)]
    #[schemars(with = "std::collections::BTreeMap<String, String>")]
    pub questions: IndexMap<String, String>,
    /// The P(yes) every question must reach, from 0 to 1. Default 0.8.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    /// Failed evaluations allowed before the phase is blocked, 1 to 5.
    /// Default 3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_attempts: Option<u32>,
    /// The decision model; none means the chat's default decision model,
    /// else the configured one, else the chat's own language model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<std::collections::BTreeMap<String, String>>")]
    pub model: Option<DecisionModelRef>,
    /// Accept verbalized (uncalibrated) probabilities. Default false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_uncalibrated: Option<bool>,
}

/// The checks every gate gets, whoever declares it: 1 to 8 yes/no questions,
/// a threshold from 0 to 1 and 1 to 5 attempts. The agent's floor
/// ([`MIN_AGENT_GATE_THRESHOLD`]) is applied where the plan is written,
/// because a gate the agent repeats unchanged keeps whatever its owner set.
fn build_gate(
    phase_id: &str,
    questions: &IndexMap<String, String>,
    threshold: Option<f64>,
    max_attempts: Option<u32>,
    model: Option<DecisionModelRef>,
    allow_uncalibrated: Option<bool>,
    owner: GateOwner,
) -> Result<PhaseGate, String> {
    let count = questions.len();
    if count == 0 || count > MAX_GATE_QUESTIONS {
        return Err(format!(
            "Phase \"{phase_id}\"'s gate needs 1 to {MAX_GATE_QUESTIONS} yes/no questions, not \
             {count}."
        ));
    }
    let questions: IndexMap<String, Question> = questions
        .iter()
        .map(|(name, text)| {
            (
                name.trim().to_string(),
                Question {
                    instructions: text.trim().to_string(),
                    kind: DecisionKind::Binary {},
                    threshold: None,
                },
            )
        })
        .collect();
    validate_questions(&questions).map_err(|e| format!("Phase \"{phase_id}\"'s gate: {e}"))?;
    let threshold = threshold.unwrap_or(DEFAULT_GATE_THRESHOLD);
    if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
        return Err(format!(
            "Phase \"{phase_id}\"'s gate threshold must be a probability from 0 to 1."
        ));
    }
    let max_attempts = max_attempts.unwrap_or_else(default_gate_attempts);
    if !(1..=MAX_GATE_ATTEMPTS).contains(&max_attempts) {
        return Err(format!(
            "Phase \"{phase_id}\"'s gate may allow 1 to {MAX_GATE_ATTEMPTS} attempts, not \
             {max_attempts}."
        ));
    }
    if let Some(model) = &model {
        if model.provider.trim().is_empty() || model.model.trim().is_empty() {
            return Err(format!(
                "Phase \"{phase_id}\"'s gate model needs both a provider and a model, for \
                 example ollama_decision/tev1:0.8b."
            ));
        }
    }
    Ok(PhaseGate {
        questions,
        threshold,
        model,
        max_attempts,
        allow_uncalibrated: allow_uncalibrated.unwrap_or(false),
        owner,
    })
}

/// Validate a declared gate into the stored [`PhaseGate`], owned by the agent.
/// The agent's threshold floor is checked where the plan is written.
pub(crate) fn validate_gate(phase_id: &str, input: &PhaseGateInput) -> Result<PhaseGate, String> {
    let model = match input.model.as_deref().map(str::trim) {
        Some(model) if !model.is_empty() => {
            Some(DecisionModelRef::parse(model).ok_or_else(|| {
                format!(
                    "Phase \"{phase_id}\"'s gate model is provider/model, for example \
                 ollama_decision/tev1:0.8b, not `{model}`."
                )
            })?)
        }
        _ => None,
    };
    build_gate(
        phase_id,
        &input.questions,
        input.threshold,
        input.max_attempts,
        model,
        input.allow_uncalibrated,
        GateOwner::Agent,
    )
}

/// Validate a gate the person sets into the stored [`PhaseGate`], owned by
/// the person. The same checks as the agent's gate, without its 0.50 floor:
/// any threshold from 0 to 1 is the person's call. Whether the chat may use
/// the gate's model is the caller's check (`DecisionService::check_model_use`).
pub fn validate_person_gate(phase_id: &str, spec: &PhaseGateSpec) -> Result<PhaseGate, String> {
    build_gate(
        phase_id.trim(),
        &spec.questions,
        spec.threshold,
        spec.max_attempts,
        spec.model.clone(),
        spec.allow_uncalibrated,
        GateOwner::Person,
    )
}

/// The same gate, whoever set it: equal questions (in any order), threshold,
/// model, attempts and calibration rule.
fn same_gate_terms(stored: &PhaseGate, declared: &PhaseGate) -> bool {
    stored.questions == declared.questions
        && stored.threshold == declared.threshold
        && stored.model == declared.model
        && stored.max_attempts == declared.max_attempts
        && stored.allow_uncalibrated == declared.allow_uncalibrated
}

/// Validate a phase's monitors as the plan declares them: at most
/// [`MAX_AGENT_MONITORS`], each one that `MonitorSpec::to_criterion` accepts,
/// and no id twice. Whether the chat may use their models is the caller's
/// check.
fn validate_phase_monitors(phase_id: &str, specs: &[MonitorSpec]) -> Result<(), String> {
    if specs.len() > MAX_AGENT_MONITORS {
        return Err(format!(
            "Phase \"{phase_id}\" may carry at most {MAX_AGENT_MONITORS} monitors, not {}.",
            specs.len()
        ));
    }
    // Any chat id will do here: the subagent's own monitor names its parent
    // when it is made at spawn time.
    const VALIDATION_OWNER: &str = "orchestration-phase";
    let mut taken = BTreeSet::new();
    for spec in specs {
        let criterion = spec
            .to_criterion(VALIDATION_OWNER, &taken)
            .map_err(|e| format!("Phase \"{phase_id}\"'s monitors: {e}"))?;
        if !taken.insert(criterion.id.clone()) {
            return Err(format!(
                "Phase \"{phase_id}\" names monitor id `{}` twice; give each monitor its own id.",
                criterion.id
            ));
        }
    }
    Ok(())
}

fn is_slug(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= MAX_PHASE_ID_CHARS
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn bounded_text(raw: &str, max: usize, what: &str) -> Result<String, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Err(format!("{what} is empty."));
    }
    let count = text.chars().count();
    if count > max {
        return Err(format!("{what} is {count} characters; the limit is {max}."));
    }
    Ok(text.to_string())
}

fn optional_text(raw: Option<&str>, max: usize, what: &str) -> Result<Option<String>, String> {
    match raw.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => bounded_text(text, max, what).map(Some),
        None => Ok(None),
    }
}

/// Who writes a plan. The agent's rules bind every author but the person, who
/// never writes a plan through these transitions; a workflow's gates have no
/// threshold floor and are owned by the workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanAuthor {
    /// The orchestrating agent, through `workspace_orchestrate`.
    Agent,
    /// A workflow's declared plan, seeded into a new chat.
    Workflow,
}

impl PlanAuthor {
    fn gate_owner(self) -> GateOwner {
        match self {
            Self::Agent => GateOwner::Agent,
            Self::Workflow => GateOwner::Workflow,
        }
    }
}

/// One declared phase as the plan transitions take it.
struct PhaseDecl<'a> {
    phase: &'a PlanPhaseInput,
    gate: Option<&'a PhaseGateInput>,
    monitors: Option<&'a [MonitorSpec]>,
}

/// One declared phase, validated on its own and against the phases listed
/// before it, before it meets its stored self ([`build_phase`]).
struct CheckedPhase {
    id: String,
    title: String,
    goal: Option<String>,
    /// Left out, an open gated phase keeps its stored kind, and any other
    /// phase is [`PhaseKind::Other`].
    kind: Option<PhaseKind>,
    depends_on: Vec<String>,
    /// Owned by the plan's author.
    gate: Option<PhaseGate>,
    /// Left out, the stored ones stay; `[]` clears them.
    monitors: Option<Vec<MonitorSpec>>,
}

/// A phase's dependencies: each an earlier phase of this plan, never the
/// phase itself, kept once each in the order given.
fn checked_dependencies(
    id: &str,
    declared: &[String],
    earlier: &[&str],
) -> Result<Vec<String>, String> {
    let mut depends_on: Vec<String> = Vec::new();
    for dependency in declared {
        let dependency = dependency.trim();
        if dependency == id {
            return Err(format!("Phase \"{id}\" cannot depend on itself."));
        }
        if !earlier.contains(&dependency) {
            return Err(format!(
                "Phase \"{id}\" depends on \"{dependency}\", which is not an earlier \
                 phase of this plan. A phase may depend only on phases listed before it: \
                 {}.",
                join_ids(earlier.iter().copied())
            ));
        }
        if !depends_on.iter().any(|d| d == dependency) {
            depends_on.push(dependency.to_string());
        }
    }
    Ok(depends_on)
}

/// Check one declared phase on its own and against the phases listed before
/// it (`earlier`): its id, title, goal, dependencies, gate and monitors, in
/// that order, so the first fault is the one named.
fn check_phase_decl(
    author: PlanAuthor,
    decl: &PhaseDecl<'_>,
    earlier: &[&str],
) -> Result<CheckedPhase, String> {
    let input = decl.phase;
    let id = input.id.trim();
    if !is_slug(id) {
        return Err(format!(
            "Phase id \"{id}\" is not a slug. Use 1 to {MAX_PHASE_ID_CHARS} lowercase \
             letters, digits or hyphens, for example \"research\" or \"verify-api\"."
        ));
    }
    if earlier.contains(&id) {
        return Err(format!(
            "Phase id \"{id}\" appears twice; ids must be unique."
        ));
    }
    let title = bounded_text(
        &input.title,
        MAX_TITLE_CHARS,
        &format!("Phase \"{id}\"'s title"),
    )?;
    let goal = optional_text(
        input.goal.as_deref(),
        MAX_GOAL_CHARS,
        &format!("Phase \"{id}\"'s goal"),
    )?;
    let depends_on = checked_dependencies(id, &input.depends_on, earlier)?;
    let kind = input.kind.as_deref().map(PhaseKind::parse_lenient);
    let gate = decl
        .gate
        .map(|gate| {
            validate_gate(id, gate).map(|gate| PhaseGate {
                owner: author.gate_owner(),
                ..gate
            })
        })
        .transpose()?;
    if let Some(monitors) = decl.monitors {
        validate_phase_monitors(id, monitors)?;
    }
    Ok(CheckedPhase {
        id: id.to_string(),
        title,
        goal,
        kind,
        depends_on,
        gate,
        monitors: decl.monitors.map(<[MonitorSpec]>::to_vec),
    })
}

/// A new gate the agent declares needs a threshold of at least
/// [`MIN_AGENT_GATE_THRESHOLD`]; a workflow's has no floor.
fn check_new_gate_floor(author: PlanAuthor, id: &str, gate: &PhaseGate) -> Result<(), String> {
    if author == PlanAuthor::Agent && gate.threshold < MIN_AGENT_GATE_THRESHOLD {
        return Err(format!(
            "Phase \"{id}\"'s gate threshold is {:.2}; a gate you declare needs at least \
             {MIN_AGENT_GATE_THRESHOLD:.2}, because below that it passes a phase the \
             decision model thinks more likely unfinished than done.",
            gate.threshold
        ));
    }
    Ok(())
}

impl CheckedPhase {
    /// The phase as first declared: pending, with nothing spawned into it.
    fn into_new(self, author: PlanAuthor) -> Result<PlanPhase, String> {
        if let Some(declared) = &self.gate {
            check_new_gate_floor(author, &self.id, declared)?;
        }
        Ok(PlanPhase {
            id: self.id,
            title: self.title,
            goal: self.goal,
            kind: self.kind.unwrap_or(PhaseKind::Other),
            status: PhaseStatus::Pending,
            depends_on: self.depends_on,
            agents: Vec::new(),
            handoff: None,
            started_at: None,
            finished_at: None,
            gate: self.gate,
            gate_state: None,
            monitors: self.monitors.unwrap_or_default(),
        })
    }

    /// The phase merged with its stored self (`previous`), which keeps its
    /// agents, status, hand-off and timestamps. A stored gate binds, and an
    /// open gated phase keeps its title, goal and kind ([`build_phase`]).
    fn revise(self, author: PlanAuthor, previous: PlanPhase) -> Result<PlanPhase, String> {
        let id = self.id;
        let (gate, gate_state) = match (&previous.gate, self.gate) {
            (Some(stored), Some(declared)) => {
                if !same_gate_terms(stored, &declared) {
                    return Err(format!(
                        "Phase \"{id}\" already has a gate{}, and a revision cannot change \
                         or remove a gate. Leave `gate` out of this phase to keep it, or ask \
                         the person to change it in the app or with `biorouter session \
                         gates`. Nothing was changed.",
                        owner_phrase(stored.owner)
                    ));
                }
                (Some(stored.clone()), previous.gate_state.clone())
            }
            (Some(stored), None) => (Some(stored.clone()), previous.gate_state.clone()),
            (None, Some(declared)) => {
                check_new_gate_floor(author, &id, &declared)?;
                (Some(declared), None)
            }
            (None, None) => (None, None),
        };
        let gated_open = previous.gate.is_some() && !previous.status.is_closed();
        let (goal, kind) = if gated_open {
            // The gate reads the title, goal and kind: an open gated
            // phase keeps them. One left out is kept, not cleared.
            let kind_changed = self.kind.is_some_and(|kind| kind != previous.kind);
            let goal_changed = self.goal.is_some() && self.goal != previous.goal;
            if self.title != previous.title || kind_changed || goal_changed {
                return Err(format!(
                    "Phase \"{id}\" has a gate, so its title, goal and kind stay as they \
                     were while it is open: its gate reads them. Nothing was changed."
                ));
            }
            (previous.goal.clone(), previous.kind)
        } else {
            (self.goal, self.kind.unwrap_or(PhaseKind::Other))
        };
        Ok(PlanPhase {
            id,
            title: self.title,
            goal,
            kind,
            depends_on: self.depends_on,
            gate,
            gate_state,
            monitors: self.monitors.unwrap_or_else(|| previous.monitors.clone()),
            ..previous
        })
    }
}

/// One declared phase, validated against the phases listed before it
/// (`earlier`) and merged with its stored self when its id persists.
///
/// A stored gate binds: a revision may repeat it unchanged or leave it out
/// (either keeps the gate, its owner and its record), and any other gate for
/// that phase is refused. While the phase is open, its title, goal and kind
/// stay as they were, because its gate reads them; a goal or kind left out
/// keeps the stored one. A new gate the agent declares needs a threshold of at
/// least [`MIN_AGENT_GATE_THRESHOLD`].
///
/// Monitors given replace the stored ones (`[]` clears them); left out, the
/// stored ones stay.
fn build_phase(
    author: PlanAuthor,
    decl: &PhaseDecl<'_>,
    earlier: &[&str],
    existing: Option<&OrchestrationPlan>,
) -> Result<PlanPhase, String> {
    let checked = check_phase_decl(author, decl, earlier)?;
    match existing.and_then(|plan| plan.phase(&checked.id)).cloned() {
        Some(previous) => checked.revise(author, previous),
        None => checked.into_new(author),
    }
}

/// [`apply_gated_plan`] with no gates declared, the shape the tests write.
#[cfg(test)]
pub(crate) fn apply_plan(
    existing: Option<OrchestrationPlan>,
    title: Option<&str>,
    goal: Option<&str>,
    phases: &[PlanPhaseInput],
) -> Result<OrchestrationPlan, String> {
    let phases: Vec<PhaseDecl<'_>> = phases
        .iter()
        .map(|phase| PhaseDecl {
            phase,
            gate: None,
            monitors: None,
        })
        .collect();
    apply_plan_inputs(PlanAuthor::Agent, existing, title, goal, &phases)
}

/// Declare a plan, or revise the stored one, as the agent.
///
/// A phase whose id is kept keeps its agents, status, hand-off and timestamps;
/// its title, goal, kind and dependencies take the new values (an open gated
/// phase keeps its title, goal and kind), its gate stays (a stored gate binds:
/// it can be repeated unchanged or left out, never changed), and its monitors
/// are the ones declared (left out, they stay). A phase that is dropped from
/// the list is removed only when nothing was ever spawned into it and no open
/// gate is on it, because its children would otherwise vanish from every
/// surface that draws the plan, and its gate would stop holding anything. A
/// phase stored after an open gated phase cannot be moved ahead of it.
pub(crate) fn apply_gated_plan(
    existing: Option<OrchestrationPlan>,
    title: Option<&str>,
    goal: Option<&str>,
    phases: &[OrchestratePhaseInput],
) -> Result<OrchestrationPlan, String> {
    apply_gated_plan_as(PlanAuthor::Agent, existing, title, goal, phases)
}

/// [`apply_gated_plan`] for a given author: the agent's rules, or a
/// workflow's (gates owned by the workflow, with no threshold floor). A stored
/// gate binds either author.
pub fn apply_gated_plan_as(
    author: PlanAuthor,
    existing: Option<OrchestrationPlan>,
    title: Option<&str>,
    goal: Option<&str>,
    phases: &[OrchestratePhaseInput],
) -> Result<OrchestrationPlan, String> {
    let phases: Vec<PhaseDecl<'_>> = phases
        .iter()
        .map(|input| PhaseDecl {
            phase: &input.phase,
            gate: input.gate.as_ref(),
            monitors: input.monitors.as_deref(),
        })
        .collect();
    apply_plan_inputs(author, existing, title, goal, &phases)
}

/// A workflow's declared plan as a new chat's FIRST plan: gates owned by the
/// workflow, which bind the agent from its first turn, with no threshold
/// floor. Revision 1, so it can be stored with [`plan_record`] in a
/// transaction of the caller's own. `_now` is the caller's clock; a plan whose
/// phases have not started records no time yet.
pub fn apply_workflow_plan(
    title: &str,
    goal: Option<&str>,
    phases: Vec<OrchestratePhaseInput>,
    _now: DateTime<Utc>,
) -> Result<OrchestrationPlan, String> {
    let mut plan = apply_gated_plan_as(PlanAuthor::Workflow, None, Some(title), goal, &phases)?;
    plan.revision = 1;
    Ok(plan)
}

/// Check a workflow's declared plan without storing it: [`apply_workflow_plan`]
/// as a dry run. `Err` is the sentence that names what is wrong.
pub fn validate_workflow_phases(
    title: &str,
    goal: Option<&str>,
    phases: &[OrchestratePhaseInput],
) -> Result<(), String> {
    apply_gated_plan_as(PlanAuthor::Workflow, None, Some(title), goal, phases).map(|_| ())
}

/// The stored form of `plan`, the value [`PLAN_STATE`]/[`PLAN_VERSION`]
/// holds, for a caller that writes it in one transaction with other keys.
pub fn plan_record(plan: &OrchestrationPlan) -> serde_json::Value {
    serde_json::to_value(plan).expect("a plan has only string keys")
}

/// The monitors every subagent spawned into `phase_id` starts with, as the
/// plan declared them. Empty for an unknown phase or one with none.
pub fn phase_monitors_for(plan: &OrchestrationPlan, phase_id: &str) -> Vec<MonitorSpec> {
    plan.phase(phase_id.trim())
        .map(|phase| phase.monitors.clone())
        .unwrap_or_default()
}

/// [`phase_monitors_for`] on the stored plan of `parent_session_id`. No plan
/// is no monitors; a plan this build cannot read is an error.
pub async fn phase_monitors(
    session_manager: &SessionManager,
    parent_session_id: &str,
    phase_id: &str,
) -> anyhow::Result<Vec<MonitorSpec>> {
    Ok(load_plan(session_manager, parent_session_id)
        .await?
        .map(|plan| phase_monitors_for(&plan, phase_id))
        .unwrap_or_default())
}

/// [`PHASE_MONITORS_NOT_RETROACTIVE`] when a revision of `existing` changes
/// the monitors of a phase that already has subagents, else `None`: what the
/// revision's reply adds.
pub fn revision_monitor_note(
    existing: Option<&OrchestrationPlan>,
    phases: &[OrchestratePhaseInput],
) -> Option<&'static str> {
    let existing = existing?;
    phases
        .iter()
        .any(|input| {
            let Some(declared) = &input.monitors else {
                return false;
            };
            existing
                .phase(input.phase.id.trim())
                .is_some_and(|stored| !stored.agents.is_empty() && &stored.monitors != declared)
        })
        .then_some(PHASE_MONITORS_NOT_RETROACTIVE)
}

/// What the stored plan of `session_id` declares, without any of its state:
/// its title and goal, and for each phase its id, title, kind, goal,
/// dependencies, gate definition and monitors. Never a hand-off, a subagent,
/// a status or a gate's record. `None` when the chat has no plan or it cannot
/// be read. For capturing a chat's plan into a workflow.
pub async fn declared_skeleton(
    manager: &SessionManager,
    session_id: &str,
) -> Option<(String, Option<String>, Vec<OrchestratePhaseInput>)> {
    match load_plan(manager, session_id).await {
        Ok(plan) => plan.map(|plan| skeleton_of(&plan)),
        Err(e) => {
            tracing::warn!(session_id, "orchestration plan unreadable: {e}");
            None
        }
    }
}

/// Pure half of [`declared_skeleton`].
fn skeleton_of(plan: &OrchestrationPlan) -> (String, Option<String>, Vec<OrchestratePhaseInput>) {
    let phases = plan
        .phases
        .iter()
        .map(|phase| OrchestratePhaseInput {
            phase: PlanPhaseInput {
                id: phase.id.clone(),
                title: phase.title.clone(),
                kind: Some(phase.kind.as_str().to_string()),
                goal: phase.goal.clone(),
                depends_on: phase.depends_on.clone(),
            },
            gate: phase.gate.as_ref().map(|gate| PhaseGateInput {
                questions: gate
                    .questions
                    .iter()
                    .map(|(name, question)| (name.clone(), question.instructions.clone()))
                    .collect(),
                threshold: Some(gate.threshold),
                max_attempts: Some(gate.max_attempts),
                allow_uncalibrated: gate.allow_uncalibrated.then_some(true),
                model: gate.model.as_ref().map(DecisionModelRef::label),
            }),
            monitors: (!phase.monitors.is_empty()).then(|| phase.monitors.clone()),
        })
        .collect();
    (plan.title.clone(), plan.goal.clone(), phases)
}

fn apply_plan_inputs(
    author: PlanAuthor,
    existing: Option<OrchestrationPlan>,
    title: Option<&str>,
    goal: Option<&str>,
    phases: &[PhaseDecl<'_>],
) -> Result<OrchestrationPlan, String> {
    let title = match (title.map(str::trim).filter(|t| !t.is_empty()), &existing) {
        (Some(title), _) => bounded_text(title, MAX_TITLE_CHARS, "The plan's title")?,
        (None, Some(plan)) => plan.title.clone(),
        (None, None) => return Err("A new plan needs a title.".to_string()),
    };
    let goal = match goal {
        Some(goal) => optional_text(Some(goal), MAX_GOAL_CHARS, "The plan's goal")?,
        None => existing.as_ref().and_then(|plan| plan.goal.clone()),
    };
    if phases.is_empty() {
        return Err("A plan needs at least one phase.".to_string());
    }
    if phases.len() > MAX_PHASES {
        return Err(format!(
            "A plan may have at most {MAX_PHASES} phases; this one has {}. Merge some \
             phases, or run the work as two plans: once every phase of the first is \
             completed or skipped, declare the second with action:\"plan\" fresh:true.",
            phases.len()
        ));
    }

    let mut seen: Vec<&str> = Vec::with_capacity(phases.len());
    let mut next_phases = Vec::with_capacity(phases.len());
    for decl in phases {
        let phase = build_phase(author, decl, &seen, existing.as_ref())?;
        seen.push(decl.phase.id.trim());
        next_phases.push(phase);
    }

    if let Some(previous) = &existing {
        // A gated phase stays in the plan until its gate passes or the person
        // decides, even one that never ran: dropping it would release every
        // phase it holds.
        let dropped_gated: Vec<&str> = previous
            .phases
            .iter()
            .filter(|phase| {
                phase.gate.is_some()
                    && !phase.status.is_closed()
                    && !seen.contains(&phase.id.as_str())
            })
            .map(|phase| phase.id.as_str())
            .collect();
        if !dropped_gated.is_empty() {
            return Err(format!(
                "This revision removes {}, which {} a gate. A gated phase stays in the plan \
                 until its gate passes or the person decides, so keep it in the list. Nothing \
                 was changed.",
                join_ids(dropped_gated.iter().copied()),
                if dropped_gated.len() == 1 {
                    "has"
                } else {
                    "have"
                }
            ));
        }
        // A phase stored after an open gated phase waits for it, and stays
        // after it.
        let new_index = |id: &str| seen.iter().position(|seen| *seen == id);
        for (gate_index, gated) in previous.phases.iter().enumerate() {
            if gated.gate.is_none() || gated.status.is_closed() {
                continue;
            }
            let Some(gate_at) = new_index(&gated.id) else {
                continue;
            };
            for later in &previous.phases[gate_index + 1..] {
                // A closed phase waits for nothing, wherever it is listed.
                if later.status.is_closed() {
                    continue;
                }
                if new_index(&later.id).is_some_and(|at| at < gate_at) {
                    return Err(format!(
                        "Phase \"{}\" waits for phase \"{}\"'s gate and cannot be moved ahead \
                         of it. Nothing was changed.",
                        later.id, gated.id
                    ));
                }
            }
        }

        let dropped: Vec<&PlanPhase> = previous
            .phases
            .iter()
            .filter(|phase| !seen.contains(&phase.id.as_str()) && !phase.agents.is_empty())
            .collect();
        if !dropped.is_empty() {
            let finished = previous.phases.iter().all(|phase| phase.status.is_closed());
            return Err(format!(
                "This revision removes {}, which already ran subagents. Keep {} in the \
                 list (you may change its title, goal or kind), or mark it skipped with \
                 action:\"complete\" status:\"skipped\".{} Nothing was changed.",
                join_ids(dropped.iter().map(|phase| phase.id.as_str())),
                if dropped.len() == 1 { "it" } else { "them" },
                if finished {
                    " Every phase of this plan is closed, so you can also start a new plan \
                     with fresh:true."
                } else {
                    ""
                }
            ));
        }
    }

    Ok(OrchestrationPlan {
        title,
        goal,
        revision: existing.map(|plan| plan.revision).unwrap_or(0),
        phases: next_phases,
    })
}

/// [`apply_fresh_gated_plan`] with no gates declared, the shape the tests write.
#[cfg(test)]
pub(crate) fn apply_fresh_plan(
    existing: Option<OrchestrationPlan>,
    title: Option<&str>,
    goal: Option<&str>,
    phases: &[PlanPhaseInput],
) -> Result<OrchestrationPlan, String> {
    let phases: Vec<OrchestratePhaseInput> = phases
        .iter()
        .map(|phase| OrchestratePhaseInput {
            phase: phase.clone(),
            gate: None,
            monitors: None,
        })
        .collect();
    apply_fresh_gated_plan(existing, title, goal, &phases)
}

/// `action:"plan"` with `fresh:true`: replace a FINISHED plan with a new one.
///
/// The way past the plan's limits ([`MAX_PLAN_AGENTS`], [`MAX_PHASES`]): a
/// revision can never drop a phase that ran subagents, so without this a chat
/// that reached either limit could never run another phase. Refused while any
/// phase is still open, so no running or pending phase disappears. The new
/// phases start empty, even where an id repeats, and receive none of the old
/// plan's hand-offs or gates; the old plan's children stay in the subagent
/// roster. The title is kept when none is given, and the revision keeps
/// counting up.
pub(crate) fn apply_fresh_gated_plan(
    existing: Option<OrchestrationPlan>,
    title: Option<&str>,
    goal: Option<&str>,
    phases: &[OrchestratePhaseInput],
) -> Result<OrchestrationPlan, String> {
    let Some(previous) = existing else {
        return apply_gated_plan(None, title, goal, phases);
    };
    if let Some((open, closing)) = closing_open_phases(&previous) {
        return Err(format!(
            "fresh:true replaces a plan only once every phase is completed or skipped; \
             still open: {open}. {closing} Or revise this plan without fresh. Nothing was \
             changed."
        ));
    }
    let title = title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or(previous.title.as_str());
    apply_gated_plan(None, Some(title), goal, phases)
}

/// Every open phase of `plan` and how the agent closes them before
/// `fresh:true`, or `None` when none is open. An ungated phase closes with
/// `action:"complete"`, skipped for work the agent will not do; a gated one
/// only through its gate or the person, because the agent may not skip it.
fn closing_open_phases(plan: &OrchestrationPlan) -> Option<(String, String)> {
    let open: Vec<&PlanPhase> = plan
        .phases
        .iter()
        .filter(|phase| !phase.status.is_closed())
        .collect();
    if open.is_empty() {
        return None;
    }
    let ids = |gated: bool| {
        join_ids(
            open.iter()
                .filter(|phase| phase.gate.is_some() == gated)
                .map(|phase| phase.id.as_str()),
        )
    };
    let gated = open.iter().filter(|phase| phase.gate.is_some()).count();
    let mut closing = Vec::new();
    if gated < open.len() {
        let which = if gated == 0 {
            "them".to_string()
        } else {
            ids(false)
        };
        closing.push(format!(
            "Close {which} with action:\"complete\" (status:\"skipped\" for work you will not \
             do)."
        ));
    }
    if gated > 0 {
        let one = gated == 1;
        closing.push(format!(
            "{} {} {}, so {} only through {} or the person, who can pass or skip {} in the app \
             or with `biorouter session gates`.",
            if one { "Phase" } else { "Phases" },
            ids(true),
            if one { "has a gate" } else { "have gates" },
            if one { "it closes" } else { "they close" },
            if one { "its gate" } else { "their gates" },
            if one { "it" } else { "them" },
        ));
    }
    Some((
        join_ids(open.iter().map(|phase| phase.id.as_str())),
        closing.join(" "),
    ))
}

/// `action:"start"`: a pending or blocked phase moves to in progress. Returns
/// the plan and a sentence for the model. Refused for a phase an earlier open
/// gate holds.
pub(crate) fn apply_start(
    plan: Option<OrchestrationPlan>,
    phase_id: &str,
    now: DateTime<Utc>,
) -> Result<(OrchestrationPlan, String), String> {
    let mut plan = plan.ok_or_else(|| NO_PLAN.to_string())?;
    let unknown = plan.unknown_phase(phase_id);
    let not_completed: Vec<String> = {
        let phase = plan.phase(phase_id).ok_or(unknown)?;
        if !phase.status.is_closed() {
            if let Some(refusal) = plan.held_refusal(phase_id) {
                return Err(refusal);
            }
        }
        phase
            .depends_on
            .iter()
            .filter(|dependency| {
                plan.phase(dependency)
                    .is_some_and(|d| d.status != PhaseStatus::Completed)
            })
            .cloned()
            .collect()
    };
    let phase = plan.phase_mut(phase_id).expect("checked above");
    let mut note = match phase.status {
        PhaseStatus::Completed | PhaseStatus::Skipped => {
            return Err(format!(
                "Phase \"{phase_id}\" is already {}, so it cannot start again. To redo its \
                 work, add a new phase (for example \"{phase_id}-2\") with action:\"plan\".",
                phase.status.as_str()
            ));
        }
        PhaseStatus::InProgress => format!("Phase \"{phase_id}\" was already in progress."),
        PhaseStatus::Pending | PhaseStatus::Blocked => {
            phase.status = PhaseStatus::InProgress;
            phase.started_at.get_or_insert(now);
            phase.finished_at = None;
            format!("Phase \"{phase_id}\" is in progress.")
        }
    };
    if !not_completed.is_empty() {
        note.push_str(&format!(
            " It depends on {}, which {} not completed yet: children spawned now receive \
             only the hand-offs written so far.",
            not_completed.join(", "),
            if not_completed.len() == 1 {
                "is"
            } else {
                "are"
            }
        ));
    }
    Ok((plan, note))
}

/// `action:"handoff"`: write (or replace) a phase's hand-off.
pub(crate) fn apply_handoff(
    plan: Option<OrchestrationPlan>,
    phase_id: &str,
    summary: &str,
    now: DateTime<Utc>,
) -> Result<OrchestrationPlan, String> {
    let mut plan = plan.ok_or_else(|| NO_PLAN.to_string())?;
    let summary = bounded_text(summary, MAX_HANDOFF_CHARS, "The hand-off summary")?;
    let unknown = plan.unknown_phase(phase_id);
    let phase = plan.phase_mut(phase_id).ok_or(unknown)?;
    phase.handoff = Some(PhaseHandoff {
        summary,
        source: HandoffSource::Parent,
        written_at: now,
    });
    Ok(plan)
}

/// One child of a phase as the barrier and the auto hand-off see it.
#[derive(Debug, Clone, PartialEq)]
pub struct PhaseAgentState {
    pub(crate) child_session_id: String,
    /// The role, or the child's name, or its id: whatever identifies it best.
    /// A withheld child is labelled by the role its parent gave it or its id,
    /// never by its session name, which a person may have renamed in its tab.
    pub(crate) label: String,
    pub(crate) status: SubagentRunStatus,
    /// Always `None` for a withheld child.
    pub(crate) result_line: Option<String>,
    /// The child's conversation is more private than the parent's (or its tier
    /// could not be read while it has something to report), so what it wrote
    /// stays out of the parent's plan.
    pub(crate) withheld: bool,
}

/// `action:"complete"`: close a phase, under the rules a gate puts on the
/// agent. A BARRIER: refused while any child in the phase is still queued,
/// running or being cancelled.
///
/// The gate rules, checked before the barrier:
/// - a phase an earlier open gate holds cannot complete with status completed
///   (it may be skipped or blocked when it has no gate of its own);
/// - a gated phase cannot be skipped unless the person's skip stands, and
///   cannot complete as completed while the person's skip stands;
/// - a gated phase may be marked blocked: its gate is not asked, uses no
///   attempt and still holds the phases after it;
/// - with the person's pass standing, completed closes it without its gate.
///
/// A gated phase completed with no standing decision of the person's closes
/// here WITHOUT asking its gate: that is the gate path's own close (its dry
/// run, and the verdict). The agent's door is [`apply_agent_complete`], which
/// refuses it.
///
/// `agents` is the set of child states gathered before the write began. A
/// child recorded into the phase after that (a spawn that raced this call)
/// was never checked, so the call is refused rather than closing the phase
/// over a child nobody looked at.
pub(crate) fn apply_complete(
    plan: Option<OrchestrationPlan>,
    phase_id: &str,
    status: PhaseStatus,
    summary: Option<&str>,
    agents: &[PhaseAgentState],
    now: DateTime<Utc>,
) -> Result<(OrchestrationPlan, String), String> {
    let plan = plan.ok_or_else(|| NO_PLAN.to_string())?;
    let (gated, owner, standing) = {
        let unknown = plan.unknown_phase(phase_id);
        let phase = plan.phase(phase_id).ok_or(unknown)?;
        if phase.status.is_closed() {
            // The barrier's own "already completed" sentence.
            return close_phase(plan, phase_id, status, summary, agents, now);
        }
        if status == PhaseStatus::Completed {
            if let Some(refusal) = plan.held_refusal(phase_id) {
                return Err(refusal);
            }
        }
        (
            phase.gate.is_some(),
            phase
                .gate
                .as_ref()
                .map(|gate| gate.owner)
                .unwrap_or_default(),
            phase.gate_state.as_ref().and_then(GateState::standing),
        )
    };
    if !gated {
        return close_phase(plan, phase_id, status, summary, agents, now);
    }
    match (status, standing) {
        (PhaseStatus::Skipped, Some(PersonGateAction::Skip)) => {
            let (plan, note) = close_by_person(plan, phase_id, PersonGateAction::Skip, |plan| {
                close_phase(plan, phase_id, status, summary, agents, now)
            })?;
            Ok((
                plan,
                format!("{note} The person skipped this phase, so it closed without its gate."),
            ))
        }
        (PhaseStatus::Skipped, _) => Err(format!(
            "Phase \"{phase_id}\" has a gate{}, so it closes only through its gate: complete it \
             with status \"completed\" to ask the gate, or ask the person, who can pass or skip \
             it in the app or with `biorouter session gates`. Nothing was changed.",
            owner_phrase(owner)
        )),
        (PhaseStatus::Completed, Some(PersonGateAction::Skip)) => {
            Err(standing_skip_refusal(phase_id))
        }
        (PhaseStatus::Completed, Some(PersonGateAction::Pass)) => {
            let (plan, note) = close_by_person(plan, phase_id, PersonGateAction::Pass, |plan| {
                close_phase(plan, phase_id, status, summary, agents, now)
            })?;
            Ok((
                plan,
                format!("{note} The person passed this phase's gate, so it completed without asking it."),
            ))
        }
        (PhaseStatus::Blocked, _) => {
            let (plan, note) = close_phase(plan, phase_id, status, summary, agents, now)?;
            let lead = format!("Phase \"{phase_id}\" is blocked.");
            let rest = note.strip_prefix(&lead).unwrap_or(&note);
            Ok((
                plan,
                format!(
                    "Phase \"{phase_id}\" is blocked. Its gate was not asked, used no attempt and \
                     still holds the phases after it: the phase completes only through its gate \
                     or the person.{rest}"
                ),
            ))
        }
        _ => close_phase(plan, phase_id, status, summary, agents, now),
    }
}

/// The refusal for completing a phase as completed while the person's skip
/// of it stands.
fn standing_skip_refusal(phase_id: &str) -> String {
    format!(
        "The person skipped phase \"{phase_id}\", so it closes only as skipped: complete it with \
         status \"skipped\" once its subagents finish. Nothing was changed."
    )
}

/// Close a gated phase on the person's standing decision: `close` runs the
/// barrier, and the gate's record says the person closed it.
fn close_by_person(
    plan: OrchestrationPlan,
    phase_id: &str,
    action: PersonGateAction,
    close: impl FnOnce(OrchestrationPlan) -> Result<(OrchestrationPlan, String), String>,
) -> Result<(OrchestrationPlan, String), String> {
    let (mut plan, note) = close(plan)?;
    if let Some(phase) = plan.phase_mut(phase_id) {
        let state = phase.gate_state.get_or_insert_with(GateState::default);
        state.closed_by_person = Some(action);
        state.person = None;
    }
    Ok((plan, note))
}

/// The agent's `complete` on a path that does not ask a gate: [`apply_complete`]
/// with its gate rules, and a gated phase completed as completed is refused
/// unless the person's pass stands, because only its gate (or the person) may
/// close it. Use [`gate_decision_needed`] first to tell which path a
/// completion takes.
pub fn apply_agent_complete(
    plan: Option<OrchestrationPlan>,
    phase_id: &str,
    status: PhaseStatus,
    summary: Option<&str>,
    agents: &[PhaseAgentState],
    now: DateTime<Utc>,
) -> Result<(OrchestrationPlan, String), String> {
    if status == PhaseStatus::Completed {
        if let Some(stored) = plan.as_ref() {
            if let Some(phase) = stored.phase(phase_id).filter(|p| !p.status.is_closed()) {
                // An earlier gate holding the phase is the reason it cannot
                // complete, whether or not it has a gate of its own.
                if let Some(refusal) = stored.held_refusal(phase_id) {
                    return Err(refusal);
                }
                let standing = phase.gate_state.as_ref().and_then(GateState::standing);
                if let Some(gate) = &phase.gate {
                    if standing.is_none() {
                        return Err(format!(
                            "Phase \"{phase_id}\" has a gate{}: completing it asks the gate, \
                             which this call did not do. Nothing was changed.",
                            owner_phrase(gate.owner)
                        ));
                    }
                }
            }
        }
    }
    apply_complete(plan, phase_id, status, summary, agents, now)
}

/// Which path the agent's `complete` (status completed) on `phase_id` takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatePath {
    /// Refused before anything is asked, with this sentence: an earlier gate
    /// holds the phase, or the person's skip of it stands.
    Held(String),
    /// Close without asking the gate, through [`apply_agent_complete`]: the
    /// person's pass stands (or the phase has no gate to ask, or the plan or
    /// phase is missing, which [`apply_agent_complete`] refuses).
    StandingPass,
    /// Ask the gate.
    Ask,
}

/// What the agent's `complete` with status completed must do on `phase_id`,
/// read from the stored plan before any decision model is asked.
pub fn gate_decision_needed(plan: &OrchestrationPlan, phase_id: &str) -> GatePath {
    let phase_id = phase_id.trim();
    let Some(phase) = plan.phase(phase_id) else {
        return GatePath::StandingPass;
    };
    if phase.status.is_closed() {
        return GatePath::StandingPass;
    }
    if let Some(refusal) = plan.held_refusal(phase_id) {
        return GatePath::Held(refusal);
    }
    if phase.gate.is_none() {
        return GatePath::StandingPass;
    }
    match phase.gate_state.as_ref().and_then(GateState::standing) {
        Some(PersonGateAction::Pass) => GatePath::StandingPass,
        Some(PersonGateAction::Skip) => GatePath::Held(standing_skip_refusal(phase_id)),
        None => GatePath::Ask,
    }
}

/// The barrier and the close itself, with no gate rule: what every door
/// that has already applied its own rules calls (the agent's through
/// [`apply_complete`], the gate's verdict, and the person's pass or skip,
/// which ignores holds).
fn close_phase(
    plan: OrchestrationPlan,
    phase_id: &str,
    status: PhaseStatus,
    summary: Option<&str>,
    agents: &[PhaseAgentState],
    now: DateTime<Utc>,
) -> Result<(OrchestrationPlan, String), String> {
    let mut plan = plan;
    if !matches!(
        status,
        PhaseStatus::Completed | PhaseStatus::Blocked | PhaseStatus::Skipped
    ) {
        return Err(
            "status must be \"completed\" (the default), \"blocked\" or \"skipped\".".to_string(),
        );
    }
    let summary = optional_text(summary, MAX_HANDOFF_CHARS, "The hand-off summary")?;
    let unknown = plan.unknown_phase(phase_id);
    let phase = plan.phase_mut(phase_id).ok_or(unknown)?;
    if phase.status.is_closed() {
        return Err(format!(
            "Phase \"{phase_id}\" is already {}. To change what it hands forward, use \
             action:\"handoff\".",
            phase.status.as_str()
        ));
    }

    let checked: HashSet<&str> = agents
        .iter()
        .map(|agent| agent.child_session_id.as_str())
        .collect();
    let unchecked: Vec<&str> = phase
        .agents
        .iter()
        .map(|agent| agent.child_session_id.as_str())
        .filter(|id| !checked.contains(id))
        .collect();
    if !unchecked.is_empty() {
        return Err(format!(
            "A subagent was spawned into phase \"{phase_id}\" while this call ran ({}). \
             Watch it with workspace_watch first, then complete the phase. Nothing was \
             changed.",
            unchecked.join(", ")
        ));
    }

    let active: Vec<&PhaseAgentState> = agents
        .iter()
        .filter(|agent| agent.status.is_active())
        .collect();
    if !active.is_empty() {
        let ids: Vec<String> = active
            .iter()
            .map(|agent| format!("\"{}\"", agent.child_session_id))
            .collect();
        let described: Vec<String> = active
            .iter()
            .map(|agent| {
                format!(
                    "{} ({}, {})",
                    agent.child_session_id,
                    agent.label,
                    run_status_word(agent.status)
                )
            })
            .collect();
        return Err(format!(
            "Phase \"{phase_id}\" cannot be completed yet: {} of its subagents {} still \
             working: {}. Wait for them with workspace_watch {{\"session_ids\": [{}], \
             \"mode\": \"all\"}}, or stop them with workspace_close, then complete the \
             phase. Nothing was changed.",
            active.len(),
            if active.len() == 1 { "is" } else { "are" },
            described.join("; "),
            ids.join(", ")
        ));
    }

    let mut note = format!("Phase \"{phase_id}\" is {}.", status.as_str());
    let withheld = agents.iter().filter(|agent| agent.withheld).count();
    if let Some(summary) = summary {
        phase.handoff = Some(PhaseHandoff {
            summary,
            source: HandoffSource::Parent,
            written_at: now,
        });
        note.push_str(" Its hand-off is the summary you gave.");
    } else if phase.handoff.is_none() && status == PhaseStatus::Completed {
        if let Some(composed) = compose_auto_handoff(agents) {
            phase.handoff = Some(PhaseHandoff {
                summary: composed,
                source: HandoffSource::Auto,
                written_at: now,
            });
            note.push_str(
                " You wrote no hand-off, so one was composed from its subagents' result \
                 lines; replace it with action:\"handoff\" if it misses what matters.",
            );
            if withheld > 0 {
                note.push(' ');
                note.push_str(&withheld_sentence(withheld));
            }
        }
    }
    phase.status = status;
    phase.finished_at = Some(now);
    Ok((plan, note))
}

fn run_status_word(status: SubagentRunStatus) -> &'static str {
    match status {
        SubagentRunStatus::Queued => "queued",
        SubagentRunStatus::Running => "running",
        SubagentRunStatus::Cancelling => "cancelling",
        SubagentRunStatus::Completed => "completed",
        SubagentRunStatus::Blocked => "blocked",
        SubagentRunStatus::Incomplete => "incomplete",
        SubagentRunStatus::Failed => "failed",
        SubagentRunStatus::Interrupted => "interrupted",
        SubagentRunStatus::Unknown => "unknown",
    }
}

/// How many result lines a hand-off left out, and why. Said in the hand-off
/// itself and in the reply to `complete`, so neither the parent nor a later
/// phase mistakes a withheld result for a child that reported nothing.
fn withheld_sentence(count: usize) -> String {
    if count == 1 {
        "1 result was withheld for privacy: that subagent's conversation may hold data \
         more private than this chat's."
            .to_string()
    } else {
        format!(
            "{count} results were withheld for privacy: those subagents' conversations may \
             hold data more private than this chat's."
        )
    }
}

/// The hand-off a phase gets when the parent completes it without writing one:
/// one line per child, its status and the first line of what it reported.
/// `None` when the phase ran no children.
///
/// A withheld child keeps its line (its label, status and id are the parent's
/// own knowledge) but not its result, and the count of withheld results comes
/// right after the heading, where the length cap cannot cut it off.
pub(crate) fn compose_auto_handoff(agents: &[PhaseAgentState]) -> Option<String> {
    if agents.is_empty() {
        return None;
    }
    let mut text = format!(
        "Composed from the result lines of the phase's {} subagent{}:",
        agents.len(),
        if agents.len() == 1 { "" } else { "s" }
    );
    let withheld = agents.iter().filter(|agent| agent.withheld).count();
    if withheld > 0 {
        text.push(' ');
        text.push_str(&withheld_sentence(withheld));
    }
    for agent in agents {
        let line = if agent.withheld {
            "(result withheld)"
        } else {
            agent
                .result_line
                .as_deref()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .unwrap_or("(reported no result line; read it with workspace_read_conversation)")
        };
        text.push_str(&format!(
            "\n- {} [{}] ({}): {}",
            agent.label,
            run_status_word(agent.status),
            agent.child_session_id,
            line
        ));
    }
    Some(truncate_chars(&text, MAX_HANDOFF_CHARS))
}

/// Shorten to at most `max` characters, marking the cut.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    const MARK: &str = " [shortened]";
    let keep = max.saturating_sub(MARK.chars().count());
    let mut out: String = text.chars().take(keep).collect();
    out.push_str(MARK);
    out
}

/// The hand-offs a child spawned into `phase_id` receives, in plan order: the
/// phases it names in `depends_on`, or every completed earlier phase when it
/// names none.
fn handoff_sources<'a>(plan: &'a OrchestrationPlan, phase_id: &str) -> Vec<&'a PlanPhase> {
    let Some(index) = plan.phases.iter().position(|phase| phase.id == phase_id) else {
        return Vec::new();
    };
    let phase = &plan.phases[index];
    if phase.depends_on.is_empty() {
        plan.phases[..index]
            .iter()
            .filter(|earlier| earlier.status == PhaseStatus::Completed && earlier.handoff.is_some())
            .collect()
    } else {
        plan.phases
            .iter()
            .filter(|candidate| {
                phase.depends_on.contains(&candidate.id) && candidate.handoff.is_some()
            })
            .collect()
    }
}

pub(crate) fn render_handoff_context(plan: &OrchestrationPlan, phase_id: &str) -> Option<String> {
    let sources = handoff_sources(plan, phase_id);
    if sources.is_empty() {
        return None;
    }
    let mut text = String::from(
        "## Results from earlier phases\n\n\
         The agent that delegated to you runs this work in phases. These are the results \
         its earlier phases handed forward. Build on them; your task above still decides \
         what you do.",
    );
    for source in sources {
        let handoff = source.handoff.as_ref().expect("filtered on handoff");
        text.push_str(&format!(
            "\n\n### {} ({}, {})\n\n{}",
            source.title,
            source.id,
            source.status.as_str(),
            handoff.summary.trim()
        ));
    }
    Some(truncate_chars(&text, HANDOFF_CONTEXT_MAX_CHARS))
}

/// Pure half of [`snapshot`].
pub(crate) fn snapshot_of(plan: &OrchestrationPlan, rows: &[SubagentRow]) -> OrchestrationSnapshot {
    let phases = plan
        .phases
        .iter()
        .enumerate()
        .map(|(index, phase)| {
            let mut agent_ids = Vec::new();
            let mut counts = PhaseCounts::default();
            for agent in &phase.agents {
                let Some(row) = rows
                    .iter()
                    .find(|row| row.session_id == agent.child_session_id)
                else {
                    // Not one of this chat's children (a copied plan, or a child
                    // whose row is gone): it is not drawn and not counted.
                    continue;
                };
                if agent_ids.contains(&row.session_id) {
                    continue;
                }
                agent_ids.push(row.session_id.clone());
                counts.total += 1;
                match row.status {
                    SubagentRunStatus::Completed => counts.done += 1,
                    SubagentRunStatus::Queued
                    | SubagentRunStatus::Running
                    | SubagentRunStatus::Cancelling => counts.running += 1,
                    SubagentRunStatus::Blocked => counts.blocked += 1,
                    SubagentRunStatus::Failed
                    | SubagentRunStatus::Incomplete
                    | SubagentRunStatus::Interrupted => counts.failed += 1,
                    SubagentRunStatus::Unknown => {}
                }
                if row.status.is_active()
                    && row
                        .quiet_seconds
                        .is_some_and(|quiet| quiet >= QUIET_AFTER_SECONDS)
                {
                    counts.quiet += 1;
                }
            }
            PhaseSnapshot {
                id: phase.id.clone(),
                title: phase.title.clone(),
                goal: phase.goal.clone(),
                kind: phase.kind,
                status: phase.status,
                depends_on: phase.depends_on.clone(),
                agent_ids,
                counts,
                handoff: phase.handoff.as_ref().map(|handoff| {
                    truncate_chars(handoff.summary.trim(), SNAPSHOT_HANDOFF_MAX_CHARS)
                }),
                started_at: phase.started_at,
                finished_at: phase.finished_at,
                gate: gate_snapshot(plan, index),
                held_by: held_by(plan, index).map(str::to_string),
            }
        })
        .collect();
    OrchestrationSnapshot {
        title: plan.title.clone(),
        goal: plan.goal.clone(),
        revision: plan.revision,
        phases,
    }
}

/// One phase's progress in a few words: "3/4 done, 1 running, 1 quiet".
pub fn progress_words(counts: &PhaseCounts) -> String {
    let mut words = vec![format!("{}/{} done", counts.done, counts.total)];
    for (count, word) in [
        (counts.running, "running"),
        (counts.quiet, "quiet"),
        (counts.blocked, "blocked"),
        (counts.failed, "failed"),
    ] {
        if count > 0 {
            words.push(format!("{count} {word}"));
        }
    }
    words.join(", ")
}

/// The plan as the model reads it in a tool reply.
pub fn render_snapshot_text(snapshot: &OrchestrationSnapshot) -> String {
    let mut text = format!(
        "Plan \"{}\" (revision {})",
        snapshot.title, snapshot.revision
    );
    if let Some(goal) = &snapshot.goal {
        text.push_str(&format!("\nGoal: {goal}"));
    }
    for (index, phase) in snapshot.phases.iter().enumerate() {
        text.push_str(&format!(
            "\n{}. {} [{}] \"{}\": {}, {}",
            index + 1,
            phase.id,
            phase.kind.as_str(),
            phase.title,
            phase.status.as_str(),
            progress_words(&phase.counts)
        ));
        if !phase.depends_on.is_empty() {
            text.push_str(&format!("; depends on {}", phase.depends_on.join(", ")));
        }
        if let Some(holder) = &phase.held_by {
            text.push_str(&format!("; waits for {holder}'s gate"));
        }
        if !phase.agent_ids.is_empty() {
            text.push_str(&format!("\n   subagents: {}", phase.agent_ids.join(", ")));
        }
        if let Some(handoff) = &phase.handoff {
            let first: String = handoff
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(160)
                .collect();
            text.push_str(&format!("\n   hand-off: {first}"));
        }
        if let Some(gate) = &phase.gate {
            text.push_str(&format!("\n   gate: {}", gate_words(gate)));
            if !phase.status.is_closed() {
                match gate.person_decision {
                    Some(PersonGateAction::Pass) => text.push_str(
                        "\n   the person passed this gate: once its subagents finish, complete \
                         the phase with status \"completed\" and it closes without asking the \
                         gate",
                    ),
                    Some(PersonGateAction::Skip) => text.push_str(
                        "\n   the person skipped this phase: once its subagents finish, \
                         complete it with status \"skipped\"",
                    ),
                    None if gate.needs_person => {
                        text.push_str(&blocked_gate_guidance(gate, phase.held_by.as_deref()))
                    }
                    None => {}
                }
            }
        }
    }
    text
}

/// The plan text's line for an open gated phase that waits for the person
/// ([`PhaseGateSnapshot::needs_person`]) and has no standing decision. Only a
/// gate that failed every attempt leaves the agent no way on: its completion
/// is refused ([`refuse_exhausted_gate`]). After an error block or the
/// agent's own block, completing the phase asks the gate again, as the notes
/// the agent read at the time said.
fn blocked_gate_guidance(gate: &PhaseGateSnapshot, held_by: Option<&str>) -> String {
    let exhausted = gate.attempts >= gate.max_attempts && gate.last_passed == Some(false);
    if exhausted {
        return "\n   only the person can move this phase on now: they can retry, change, \
                remove, pass or skip its gate"
            .to_string();
    }
    if gate.blocked {
        let and_hold = held_by
            .map(|holder| format!(" and {holder}'s gate is through"))
            .unwrap_or_default();
        format!(
            "\n   blocked because its gate could not be evaluated {GATE_ERRORS_TO_BLOCK} times in \
             a row: complete the phase again to ask it once the model that judges it can be \
             reached{and_hold}, or the person can retry, change, remove, pass or skip its gate"
        )
    } else {
        let once_hold = held_by
            .map(|holder| format!(" once {holder}'s gate is through"))
            .unwrap_or_default();
        format!(
            "\n   marked blocked, so its gate was not asked: complete the phase with status \
             \"completed\" to ask it{once_hold}, or the person can change, remove, pass or skip \
             its gate"
        )
    }
}

/// A gate's state in a few words: "2 questions at P(yes) >= 0.80 with
/// joey-1 · Baseten, 1 of 3 attempts used, last not passed (lowest P(yes)
/// 0.42), set by the agent, holds verify". A gate "passed" or was "not
/// passed" (R1, live QA 2026-11-10), never "failed", which a person reads as
/// an error. A gate that names its decision model says which, as a person
/// reads it (TRIAGE R3, live QA M2-4), so the plan shows which model will
/// read the chat; one that names none runs on the chat's default.
pub fn gate_words(gate: &PhaseGateSnapshot) -> String {
    let model = gate
        .model
        .as_deref()
        .map(
            |label| match crate::decision::DecisionModelRef::parse(label) {
                Some(model) => format!(" with {}", crate::decision::monitor::model_phrase(&model)),
                None => format!(" with {label}"),
            },
        )
        .unwrap_or_default();
    let mut words = format!(
        "{} question{} at P(yes) >= {:.2}{model}, {} of {} attempts used",
        gate.question_count,
        if gate.question_count == 1 { "" } else { "s" },
        gate.threshold,
        gate.attempts,
        gate.max_attempts
    );
    match (gate.last_passed, gate.last_min_p) {
        (Some(true), _) => words.push_str(", passed"),
        (Some(false), Some(p)) => {
            words.push_str(&format!(", last not passed (lowest P(yes) {p:.2})"))
        }
        (Some(false), None) => words.push_str(", last not passed"),
        (None, _) => {}
    }
    if gate.blocked {
        words.push_str(", blocked");
    }
    words.push_str(", ");
    words.push_str(gate.owner.set_by_words());
    match gate.person_decision {
        Some(PersonGateAction::Pass) => words.push_str(", passed by the person"),
        Some(PersonGateAction::Skip) => words.push_str(", skipped by the person"),
        None => {}
    }
    if !gate.holds.is_empty() {
        words.push_str(&format!(", holds {}", gate.holds.join(", ")));
    }
    words
}

/// The plan as the model sees it at the top of every turn: one line per phase.
pub fn render_moim(snapshot: &OrchestrationSnapshot) -> String {
    let mut text = format!(
        "Orchestration plan \"{}\" (revision {}), driven with workspace_orchestrate:",
        snapshot.title, snapshot.revision
    );
    for phase in &snapshot.phases {
        text.push_str(&format!(
            "\n- {} ({}): {}, {}",
            phase.id,
            phase.kind.as_str(),
            phase.status.as_str().replace('_', " "),
            progress_words(&phase.counts)
        ));
        if let Some(gate) = &phase.gate {
            text.push_str(&format!(
                ", gated (attempt {} of {}{}{}{})",
                gate.attempts,
                gate.max_attempts,
                if gate.blocked { ", blocked" } else { "" },
                match gate.owner {
                    GateOwner::Agent => "",
                    GateOwner::Person => ", set by the person",
                    GateOwner::Workflow => ", set by the workflow",
                },
                match gate.person_decision {
                    Some(PersonGateAction::Pass) => ", passed by the person",
                    Some(PersonGateAction::Skip) => ", skipped by the person",
                    None => "",
                }
            ));
        }
        if let Some(holder) = &phase.held_by {
            text.push_str(&format!(", waits for {holder}'s gate"));
        }
    }
    text.push_str(
        "\nSpawn into a phase with subagent phase:\"<id>\"; complete a phase only after \
         workspace_watch mode:\"all\" on its subagents.",
    );
    truncate_chars(&text, 2_000)
}

// ---------------------------------------------------------------------------
// The contract the spawn path and the roster call.
// ---------------------------------------------------------------------------

/// Refuse a spawn into `phase_id` before any child session exists: an unknown
/// phase, one already completed or skipped, or one an earlier open gate holds.
/// `Err` carries the sentence the model reads.
pub async fn validate_spawn_phase(
    session_manager: &SessionManager,
    parent_session_id: &str,
    phase_id: &str,
) -> Result<(), String> {
    validate_spawn_phase_classified(session_manager, parent_session_id, phase_id)
        .await
        .map_err(|refusal| refusal.sentence)
}

/// A refused spawn into a phase, with what kind of refusal it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnPhaseRefusal {
    /// The sentence the model reads.
    pub sentence: String,
    /// True when the call itself named nothing (no plan, or a phase not in
    /// it): the same arguments fail again until the call changes. False when
    /// the phase exists and the plan's state refuses it now (closed, held by
    /// an earlier gate, the plan's subagent limit, an unreadable plan), which
    /// the agent answers by changing the plan rather than the call. C-5 (live
    /// QA 2026-11-10): a held phase used to be reported as rejected arguments,
    /// whose guidance "Fix the call itself; the same arguments will fail
    /// again" was false, since the same call succeeds once the gate passes.
    pub fix_the_call: bool,
}

/// [`validate_spawn_phase`], keeping whether the refusal is about the call's
/// arguments or about the plan's state at this moment.
pub async fn validate_spawn_phase_classified(
    session_manager: &SessionManager,
    parent_session_id: &str,
    phase_id: &str,
) -> Result<(), SpawnPhaseRefusal> {
    let plan = load_plan(session_manager, parent_session_id)
        .await
        .map_err(|e| SpawnPhaseRefusal {
            sentence: format!(
                "Could not read this chat's orchestration plan: {e}. Nothing was created."
            ),
            fix_the_call: false,
        })?;
    classify_spawn_phase(plan.as_ref(), phase_id)
}

/// Pure half of [`validate_spawn_phase_classified`].
pub(crate) fn classify_spawn_phase(
    plan: Option<&OrchestrationPlan>,
    phase_id: &str,
) -> Result<(), SpawnPhaseRefusal> {
    check_spawn_phase(plan, phase_id).map_err(|sentence| SpawnPhaseRefusal {
        sentence,
        fix_the_call: plan.is_none_or(|plan| plan.phase(phase_id.trim()).is_none()),
    })
}

/// Pure half of [`validate_spawn_phase`].
pub(crate) fn check_spawn_phase(
    plan: Option<&OrchestrationPlan>,
    phase_id: &str,
) -> Result<(), String> {
    let phase_id = phase_id.trim();
    let Some(plan) = plan else {
        return Err(format!(
            "This chat has no orchestration plan, so phase \"{phase_id}\" names nothing. \
             Declare phases first with workspace_orchestrate action:\"plan\", or spawn \
             without a phase. Nothing was created."
        ));
    };
    let Some(phase) = plan.phase(phase_id) else {
        return Err(format!(
            "Phase \"{phase_id}\" is not in this chat's plan. Phases you can spawn into: \
             {}. Nothing was created.",
            plan.open_phase_ids()
        ));
    };
    if phase.status.is_closed() {
        return Err(format!(
            "Phase \"{phase_id}\" is already {}, so nothing more can be spawned into it. \
             Phases you can spawn into: {}. Nothing was created.",
            phase.status.as_str(),
            plan.open_phase_ids()
        ));
    }
    if let Some(refusal) = plan.held_refusal(phase_id) {
        return Err(format!("{refusal} Nothing was created."));
    }
    if plan.agent_count() >= MAX_PLAN_AGENTS {
        // `phase_id` passed the closed check above, so a phase is open. Every
        // open phase is named, held ones too: fresh:true needs them all closed.
        let (open, closing) = closing_open_phases(plan).unwrap_or_default();
        return Err(format!(
            "This plan already has {MAX_PLAN_AGENTS} subagents, its limit. For further \
             phased work, close its open phases ({open}), then start a new plan with \
             workspace_orchestrate action:\"plan\" fresh:true. {closing} Or spawn without a \
             phase. Nothing was created."
        ));
    }
    Ok(())
}

/// The "Results from earlier phases" block appended to the instructions of a
/// child spawned into `phase_id`, or `None` when there is nothing to hand off.
pub async fn handoff_context_for(
    session_manager: &SessionManager,
    parent_session_id: &str,
    phase_id: &str,
) -> Option<String> {
    let plan = match load_plan(session_manager, parent_session_id).await {
        Ok(plan) => plan?,
        Err(e) => {
            tracing::warn!(
                parent_session_id,
                phase_id,
                "orchestration plan unreadable; spawning without hand-offs: {e}"
            );
            return None;
        }
    };
    render_handoff_context(&plan, phase_id.trim())
}

/// Pure half of [`record_spawn`].
pub(crate) fn apply_record_spawn(
    plan: Option<OrchestrationPlan>,
    phase_id: &str,
    child_session_id: &str,
    role: Option<&str>,
    now: DateTime<Utc>,
) -> Result<OrchestrationPlan, String> {
    let mut plan = plan.ok_or_else(|| NO_PLAN.to_string())?;
    let already = plan.phases.iter().any(|phase| {
        phase
            .agents
            .iter()
            .any(|agent| agent.child_session_id == child_session_id)
    });
    if already {
        return Ok(plan);
    }
    if plan.agent_count() >= MAX_PLAN_AGENTS {
        return Err(format!(
            "This plan already records {MAX_PLAN_AGENTS} subagents, its limit."
        ));
    }
    // The spawn checked the phase before its child existed; a `complete` may
    // have closed it since, or the person may have set a gate on an earlier
    // phase. A closed or held phase never takes another child.
    let held = plan.held_refusal(phase_id);
    let unknown = plan.unknown_phase(phase_id);
    let phase = plan.phase_mut(phase_id).ok_or(unknown)?;
    if phase.status.is_closed() {
        return Err(format!(
            "Phase \"{phase_id}\" was {} while this subagent was being created, so nothing \
             more can be spawned into it.",
            phase.status.as_str()
        ));
    }
    if let Some(refusal) = held {
        return Err(refusal);
    }
    let role = role
        .map(str::trim)
        .filter(|role| !role.is_empty())
        .map(|role| role.chars().take(MAX_ROLE_CHARS).collect::<String>());
    phase.agents.push(PlanAgent {
        child_session_id: child_session_id.to_string(),
        role,
        spawned_at: now,
    });
    // A spawn into a blocked phase resumes it, like `start` would.
    if matches!(phase.status, PhaseStatus::Pending | PhaseStatus::Blocked) {
        phase.status = PhaseStatus::InProgress;
        phase.started_at.get_or_insert(now);
        phase.finished_at = None;
    }
    Ok(plan)
}

/// Record that `child_session_id` was spawned into `phase_id` (moving a pending
/// phase to in progress). Called once the child's row exists and BEFORE the
/// child runs, under the spawn's [`begin_phase_spawn`] guard: an error (the
/// phase closed or was dropped since the check, or a parallel spawn took the
/// plan's last place) refuses the spawn, so no child runs outside the plan its
/// parent named.
pub async fn record_spawn(
    session_manager: &SessionManager,
    parent_session_id: &str,
    phase_id: &str,
    child_session_id: &str,
    role: Option<&str>,
) -> anyhow::Result<()> {
    let now = Utc::now();
    mutate_plan(session_manager, parent_session_id, |plan| {
        apply_record_spawn(plan, phase_id.trim(), child_session_id, role, now)
            .map(|plan| (plan, ()))
    })
    .await
}

// ---------------------------------------------------------------------------
// Spawns in flight. A spawn into a phase and a `complete` of that phase run in
// the same process (the parent's own tool batch), so a process-local registry is
// enough to order them. A spawn holds a guard from before it checks the phase
// until its child is visible to the barrier: a registered background handle, or,
// on the blocking path, the end of the child's run. Without it, `complete` could
// read the phase's children before a starting child was recorded or before its
// handle existed, find nothing active, and close the phase over it.
// ---------------------------------------------------------------------------

/// An idle entry is kept this long, so a `complete` whose two reads straddle a
/// whole spawn still sees that one began.
const SPAWN_MARK_RETENTION: Duration = Duration::from_secs(600);
/// Idle entries are pruned only once the registry holds this many.
const SPAWN_MARK_PRUNE_AT: usize = 256;

#[derive(Debug)]
struct PhaseSpawnEntry {
    in_flight: usize,
    last_begun: u64,
    idle_since: Option<Instant>,
}

type SpawnKey = (String, String);

static PHASE_SPAWNS: LazyLock<Mutex<HashMap<SpawnKey, PhaseSpawnEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
/// Process-wide, so a pruned and re-created entry never repeats a number.
static SPAWN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn spawn_key(parent_session_id: &str, phase_id: &str) -> SpawnKey {
    (parent_session_id.to_string(), phase_id.trim().to_string())
}

fn phase_spawns() -> std::sync::MutexGuard<'static, HashMap<SpawnKey, PhaseSpawnEntry>> {
    // A panic elsewhere while the lock was held leaves plain counters behind,
    // which are still right; a guard's `Drop` must never panic on them.
    PHASE_SPAWNS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What `complete` compares before it reads a phase's children and again inside
/// its write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PhaseSpawnMark {
    /// Spawns into the phase whose child is not yet visible to the barrier.
    pub(crate) in_flight: usize,
    /// The sequence number of the latest spawn begun into the phase; 0 for none.
    pub(crate) last_begun: u64,
}

pub(crate) fn phase_spawn_mark(parent_session_id: &str, phase_id: &str) -> PhaseSpawnMark {
    phase_spawns()
        .get(&spawn_key(parent_session_id, phase_id))
        .map(|entry| PhaseSpawnMark {
            in_flight: entry.in_flight,
            last_begun: entry.last_begun,
        })
        .unwrap_or_default()
}

/// Held by a spawn into a phase; see [`begin_phase_spawn`].
#[must_use = "the phase barrier holds only while the guard is alive"]
#[derive(Debug)]
pub struct PhaseSpawnGuard {
    key: SpawnKey,
}

/// Tell the barrier that a spawn into `phase_id` has begun. Take it before the
/// phase check and keep it until the child is visible to the barrier.
pub fn begin_phase_spawn(parent_session_id: &str, phase_id: &str) -> PhaseSpawnGuard {
    let key = spawn_key(parent_session_id, phase_id);
    let mut spawns = phase_spawns();
    if spawns.len() >= SPAWN_MARK_PRUNE_AT {
        spawns.retain(|_, entry| {
            entry.in_flight > 0
                || entry
                    .idle_since
                    .is_some_and(|since| since.elapsed() < SPAWN_MARK_RETENTION)
        });
    }
    let entry = spawns.entry(key.clone()).or_insert(PhaseSpawnEntry {
        in_flight: 0,
        last_begun: 0,
        idle_since: None,
    });
    entry.in_flight += 1;
    entry.last_begun = SPAWN_SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
    entry.idle_since = None;
    PhaseSpawnGuard { key }
}

impl Drop for PhaseSpawnGuard {
    fn drop(&mut self) {
        let mut spawns = phase_spawns();
        if let Some(entry) = spawns.get_mut(&self.key) {
            entry.in_flight = entry.in_flight.saturating_sub(1);
            if entry.in_flight == 0 {
                entry.idle_since = Some(Instant::now());
            }
        }
    }
}

/// The barrier's half of the ordering: refuse to close `phase_id` while a spawn
/// into it is in flight, or when one began between `before` (read before the
/// phase's children were) and `now` (read inside the write).
pub(crate) fn refuse_racing_spawns(
    phase_id: &str,
    before: PhaseSpawnMark,
    now: PhaseSpawnMark,
) -> Result<(), String> {
    if before.in_flight > 0 || now.in_flight > 0 {
        return Err(format!(
            "Phase \"{phase_id}\" cannot be completed yet: a subagent call into it has not \
             returned. Wait for that call to return, then complete the phase. Nothing was \
             changed."
        ));
    }
    if now.last_begun != before.last_begun {
        return Err(format!(
            "A subagent was spawned into phase \"{phase_id}\" while this call ran. Watch it \
             with workspace_watch first, then complete the phase. Nothing was changed."
        ));
    }
    Ok(())
}

/// The plan of `parent_session_id` with per-phase counts derived from `rows`
/// (the parent's subagent roster), or `None` when no plan was declared.
pub async fn snapshot(
    session_manager: &SessionManager,
    parent_session_id: &str,
    rows: &[SubagentRow],
) -> Option<OrchestrationSnapshot> {
    match load_plan(session_manager, parent_session_id).await {
        Ok(plan) => plan.map(|plan| snapshot_of(&plan, rows)),
        Err(e) => {
            tracing::warn!(parent_session_id, "orchestration plan unreadable: {e}");
            None
        }
    }
}

/// The parent's classification as its row records it now. Unreadable reads as
/// Public, the tier that may hold the least: a child's text then reaches the
/// plan only when the child is provably Public too.
async fn parent_classification(
    session_manager: &SessionManager,
    parent_session_id: &str,
) -> SessionClassification {
    match session_manager.get_session(parent_session_id, false).await {
        Ok(session) => session.privacy_tier,
        Err(e) => {
            tracing::warn!(
                parent_session_id,
                "parent row unreadable; withholding every child's result that is not public: {e}"
            );
            SessionClassification::Public
        }
    }
}

/// The child's classification: its roster row's, or its own session row's
/// when the roster does not list it. `None` when neither can be read.
async fn child_classification(
    session_manager: &SessionManager,
    child_session_id: &str,
    row: Option<&SubagentRow>,
) -> Option<SessionClassification> {
    match row {
        Some(row) => Some(row.privacy_tier),
        None => session_manager
            .get_session(child_session_id, false)
            .await
            .ok()
            .map(|session| session.privacy_tier),
    }
}

/// The states of a phase's children, for the barrier and the auto hand-off.
///
/// Three sources, the live one first: a child this process is running (its
/// background handle has no result yet, or a Stop-and-Send continuation is
/// pending) is active whatever else says; otherwise the roster row; otherwise a
/// finished handle's result; otherwise the child's durable outcome record. A
/// child none of them knows is `Unknown`, which the barrier does not hold on:
/// nothing in this process is running it.
///
/// Each child's tier is compared with the parent's as both rows record it
/// now. A child more private than the parent is `withheld`: its result line
/// and session name are dropped here, before anything is composed from them.
/// A child whose tier cannot be read is withheld too when it has a result line
/// to show (fail closed); with nothing to show there is nothing to withhold.
pub(crate) async fn phase_agent_states(
    session_manager: &SessionManager,
    parent_session_id: &str,
    agents: &[PlanAgent],
) -> Vec<PhaseAgentState> {
    use crate::agents::subagent_roster::session_subagents;

    let rows = match session_subagents(session_manager, parent_session_id).await {
        Ok(roster) => roster.subagents,
        Err(e) => {
            tracing::warn!(parent_session_id, "subagent roster unreadable: {e}");
            Vec::new()
        }
    };
    let handles = crate::agents::subagent_handle::list_for_session(parent_session_id);
    let parent_tier = parent_classification(session_manager, parent_session_id).await;

    let mut states = Vec::with_capacity(agents.len());
    for agent in agents {
        let id = agent.child_session_id.as_str();
        let row = rows.iter().find(|row| row.session_id == id);
        let handle = handles
            .iter()
            .rev()
            .find(|handle| handle.child_session_id == id);
        let role = agent
            .role
            .clone()
            .or_else(|| row.and_then(|row| row.role.clone()));

        let live = handle.and_then(|handle| {
            if handle.is_running() {
                Some(if handle.is_cancelled() {
                    SubagentRunStatus::Cancelling
                } else {
                    SubagentRunStatus::Running
                })
            } else if handle.continuation_pending() {
                Some(SubagentRunStatus::Running)
            } else {
                None
            }
        });

        // A finished handle's own envelope, while this process still holds it.
        let finished = handle.and_then(|handle| handle.result()).map(|result| {
            let status = match result.status {
                crate::agents::subagent_result::SubagentStatus::Completed => {
                    SubagentRunStatus::Completed
                }
                crate::agents::subagent_result::SubagentStatus::Blocked => {
                    SubagentRunStatus::Blocked
                }
                crate::agents::subagent_result::SubagentStatus::Incomplete => {
                    SubagentRunStatus::Incomplete
                }
                crate::agents::subagent_result::SubagentStatus::Error => SubagentRunStatus::Failed,
            };
            (status, first_line(&result.summary))
        });
        // The durable record, read only when nothing nearer has the answer.
        let status_known = live.is_some() || row.is_some() || finished.is_some();
        let line_known = row.is_some_and(|row| row.result_line.is_some())
            || finished.as_ref().is_some_and(|(_, line)| line.is_some());
        let recorded = if !(status_known && line_known) {
            recorded_outcome(session_manager, id).await
        } else {
            None
        };

        let status = live
            .or(row.map(|row| row.status))
            .or(finished.as_ref().map(|(status, _)| *status))
            .or(recorded.as_ref().map(|(status, _)| *status))
            .unwrap_or(SubagentRunStatus::Unknown);
        let result_line = row
            .and_then(|row| row.result_line.clone())
            .or_else(|| finished.and_then(|(_, line)| line))
            .or_else(|| recorded.and_then(|(_, line)| line));
        let child_tier = child_classification(session_manager, id, row).await;
        states.push(screened_state(
            id,
            role,
            row.map(|row| row.name.as_str()),
            status,
            result_line,
            child_tier,
            parent_tier,
        ));
    }
    states
}

/// The child's durable outcome record as (status, result line), when it has one.
async fn recorded_outcome(
    session_manager: &SessionManager,
    child_session_id: &str,
) -> Option<(SubagentRunStatus, Option<String>)> {
    use crate::agents::subagent_roster::{SUBAGENT_OUTCOME_STATE, SUBAGENT_OUTCOME_VERSION};

    let outcome = session_manager
        .get_extension_state(
            child_session_id,
            SUBAGENT_OUTCOME_STATE,
            SUBAGENT_OUTCOME_VERSION,
        )
        .await
        .ok()
        .flatten()?;
    Some((
        outcome
            .get("status")
            .cloned()
            .and_then(|status| serde_json::from_value(status).ok())
            .unwrap_or(SubagentRunStatus::Unknown),
        outcome
            .get("result_line")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    ))
}

/// One child's state with the parent's tier applied: a child more private
/// than the parent, or one whose tier is unknown while it has a result line,
/// is withheld, and loses its result line and its session name.
fn screened_state(
    child_session_id: &str,
    role: Option<String>,
    name: Option<&str>,
    status: SubagentRunStatus,
    result_line: Option<String>,
    child_tier: Option<SessionClassification>,
    parent_tier: SessionClassification,
) -> PhaseAgentState {
    let withheld = match child_tier {
        Some(child_tier) => child_tier > parent_tier,
        None => result_line.is_some(),
    };
    // A person may have renamed a withheld child in its own tab, so only the
    // role its parent gave it, or its id, labels it here.
    let name = if withheld {
        None
    } else {
        name.map(str::to_string)
    };
    let label = role
        .or(name)
        .filter(|label| !label.trim().is_empty())
        .unwrap_or_else(|| child_session_id.to_string());
    PhaseAgentState {
        child_session_id: child_session_id.to_string(),
        label,
        status,
        result_line: if withheld { None } else { result_line },
        withheld,
    }
}

fn first_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(truncate_chars(line, 300))
}

// ---------------------------------------------------------------------------
// Decision gates. `workspace_orchestrate action:"complete"` (status completed)
// on a gated phase: refuse an exhausted gate, dry-run the barrier, build the
// state, evaluate it through the decision service, then write the verdict in
// one transaction that re-checks the gate and, on a pass, the barrier.
// ---------------------------------------------------------------------------

/// The gate failed every attempt it was given.
fn gate_exhausted(phase: &PlanPhase) -> bool {
    match (&phase.gate, &phase.gate_state) {
        (Some(gate), Some(state)) => {
            state.attempts >= gate.max_attempts && state.last_passed == Some(false)
        }
        _ => false,
    }
}

/// Blocked because of its gate: every attempt failed, or the phase was blocked
/// by errors in a row.
fn gate_blocked(phase: &PlanPhase) -> bool {
    gate_exhausted(phase)
        || (phase.status == PhaseStatus::Blocked
            && phase
                .gate_state
                .as_ref()
                .is_some_and(|state| state.consecutive_errors >= GATE_ERRORS_TO_BLOCK))
}

/// The id of the gated phase that holds the OPEN phase at `index`; a closed
/// phase waits for nothing.
fn held_by(plan: &OrchestrationPlan, index: usize) -> Option<&str> {
    let phase = plan.phases.get(index)?;
    if phase.status.is_closed() {
        return None;
    }
    plan.holder_index(index)
        .map(|holder| plan.phases[holder].id.as_str())
}

fn gate_snapshot(plan: &OrchestrationPlan, index: usize) -> Option<PhaseGateSnapshot> {
    let phase = plan.phases.get(index)?;
    let gate = phase.gate.as_ref()?;
    let state = phase.gate_state.clone().unwrap_or_default();
    // A closed phase is blocked on nothing and needs no one: a gate that
    // failed every attempt before the person passed or skipped the phase
    // keeps its attempts, but nothing more can be done about it.
    let open = !phase.status.is_closed();
    let holds: Vec<String> = if !open {
        Vec::new()
    } else {
        plan.phases
            .iter()
            .enumerate()
            .skip(index + 1)
            .filter(|(later, _)| held_by(plan, *later).is_some_and(|id| id == phase.id))
            .map(|(_, later)| later.id.clone())
            .collect()
    };
    Some(PhaseGateSnapshot {
        threshold: gate.threshold,
        question_count: u32::try_from(gate.questions.len()).unwrap_or(u32::MAX),
        attempts: state.attempts,
        max_attempts: gate.max_attempts,
        last_passed: state.last_passed,
        last_min_p: state.last_min_p,
        blocked: open && gate_blocked(phase),
        owner: gate.owner,
        questions: gate
            .questions
            .iter()
            .map(|(name, question)| (name.clone(), question.instructions.clone()))
            .collect(),
        model: gate.model.as_ref().map(DecisionModelRef::label),
        last_judge: state.last_judge.clone(),
        model_adapter: gate.model.as_ref().and_then(|model| model.adapter),
        allow_uncalibrated: gate.allow_uncalibrated,
        last_error: state.last_error.clone(),
        person_decision: state.closed_by_person.or(state.standing()),
        retries_granted: state.retries_granted,
        needs_person: open && (gate_blocked(phase) || phase.status == PhaseStatus::Blocked),
        holds,
    })
}

/// What the agent is told once only the person can move a gated phase on.
const ONLY_THE_PERSON: &str = "Only they can go on from here: they can give the gate its \
     attempts again, change or remove it, or pass or skip the phase, in the app or with \
     `biorouter session gates`.";

/// The refusal for completing a phase whose gate has no attempts left, or
/// `None` when the gate may still be asked.
pub(crate) fn refuse_exhausted_gate(phase: &PlanPhase) -> Option<String> {
    if !gate_exhausted(phase) {
        return None;
    }
    let gate = phase.gate.as_ref()?;
    Some(format!(
        "Phase \"{}\"'s gate failed all {} attempts, so the phase cannot be completed through \
         it. Tell the person what the gate asked and what the results lacked. \
         {ONLY_THE_PERSON} Nothing was changed.",
        phase.id, gate.max_attempts
    ))
}

/// The one state a gate's decision model reads: the phase's goal, its hand-off
/// and, unless the hand-off was composed from them already, its children's
/// result lines. `agents` are the screened states of [`phase_agent_states`],
/// so a withheld child contributes its label and status, never its text.
pub(crate) fn gate_state_text(phase: &PlanPhase, agents: &[PhaseAgentState]) -> String {
    let mut text = format!(
        "Phase \"{}\" ({}, {}) of a delegated task.\nGoal: {}",
        phase.title,
        phase.id,
        phase.kind.as_str(),
        phase.goal.as_deref().unwrap_or("(none stated)")
    );
    match &phase.handoff {
        Some(handoff) => text.push_str(&format!("\n\nHand-off:\n{}", handoff.summary.trim())),
        None => text.push_str("\n\nHand-off: (none written)"),
    }
    let composed = phase
        .handoff
        .as_ref()
        .is_some_and(|handoff| handoff.source == HandoffSource::Auto);
    if !composed && !agents.is_empty() {
        text.push_str("\n\nSubagent results:");
        for agent in agents {
            let line = if agent.withheld {
                "(result withheld)"
            } else {
                agent
                    .result_line
                    .as_deref()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .unwrap_or("(no result line)")
            };
            text.push_str(&format!(
                "\n- {} [{}]: {}",
                agent.label,
                run_status_word(agent.status),
                line
            ));
        }
    }
    truncate_chars(&text, GATE_STATE_MAX_CHARS)
}

/// `state` narrowed to the window of the model the gate asks
/// ([`DecisionService::context_tokens_for`]), as a monitor's window is fitted
/// to its model. F1 (live QA 2026-11-10): the state was cut at a flat
/// [`GATE_STATE_MAX_CHARS`], so a gate on `tev1:0.8b` (a 2,048-token window)
/// over a phase with numeric results was refused before sending ("about 3878
/// tokens, which does not fit") and, after two such errors, blocked the phase.
/// Unchanged when the listing does not show the model; the size check still
/// judges then.
async fn fit_gate_state(
    service: &DecisionService,
    session: &DecisionSession,
    model: Option<&DecisionModelRef>,
    questions: &IndexMap<String, Question>,
    state: String,
) -> String {
    let Some(context_tokens) = service.context_tokens_for(Some(session), model).await else {
        return state;
    };
    // The fit counts with the size check's own tokenizer, without waiting.
    crate::decision::service::load_token_counter().await;
    fit_state_to_window(&state, context_tokens, questions)
}

/// `state`, kept from its start (the phase, its goal and hand-off come first),
/// narrowed until the size check's estimate of it beside `questions` fits a
/// model of `context_tokens`: first to the state budget the monitor's window
/// fit uses ([`crate::decision::service::state_budget_tokens`], at three
/// characters a token), then by the share of the overflow, because a state of
/// numbers spends a token on every digit. Never below
/// [`GATE_STATE_MIN_CHARS`].
pub(crate) fn fit_state_to_window(
    state: &str,
    context_tokens: usize,
    questions: &IndexMap<String, Question>,
) -> String {
    use crate::decision::service::{context_margin, estimate_tokens_now, state_budget_tokens};
    let floor = GATE_STATE_MIN_CHARS;
    let budget_chars = state_budget_tokens(context_tokens, questions)
        .saturating_mul(3)
        .max(floor);
    let mut text = truncate_chars(state, budget_chars.min(GATE_STATE_MAX_CHARS));
    let allowed = context_tokens.saturating_sub(context_margin(context_tokens));
    for _ in 0..GATE_FIT_ROUNDS {
        let tokens = estimate_tokens_now(&DecisionRequest {
            questions: questions.clone(),
            context: DecisionContext::Text(text.clone()),
        });
        let chars = text.chars().count();
        if tokens <= allowed || chars <= floor {
            break;
        }
        let state_tokens = estimate_tokens_now(&DecisionRequest {
            questions: IndexMap::new(),
            context: DecisionContext::Text(text.clone()),
        });
        let cut = ((tokens - allowed) * chars).div_ceil(state_tokens.max(1));
        let next = chars.saturating_sub(cut).max(floor);
        if next >= chars {
            break;
        }
        text = truncate_chars(state, next);
    }
    text
}

/// One gate question's answer.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GateAnswer {
    pub(crate) question: String,
    pub(crate) instructions: String,
    pub(crate) p_yes: f64,
    pub(crate) passed: bool,
}

/// What one evaluation of a gate came to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum GateVerdict {
    /// Every question reached the threshold.
    Passed(Vec<GateAnswer>),
    /// At least one question fell short.
    Failed(Vec<GateAnswer>),
    /// Nothing was decided: the model was refused, unreachable or unusable.
    /// Never a pass.
    Error(String),
}

/// A gate's questions as the decision service asks them, each carrying the
/// gate's threshold.
fn gate_questions(gate: &PhaseGate) -> IndexMap<String, Question> {
    gate.questions
        .iter()
        .map(|(name, question)| {
            (
                name.clone(),
                Question {
                    threshold: Some(Threshold::Yes(gate.threshold)),
                    ..question.clone()
                },
            )
        })
        .collect()
}

/// How a gate on `model` asking `questions` would be judged in the chat
/// `session`, by every check a decision makes before it sends
/// ([`DecisionService::check_model_use`]; nothing of the chat is sent). Every
/// door that stores a gate asks this: the agent's plan, the person's route
/// and CLI, and a workflow's seed.
///
/// A gate that names a model is judged by it, or refused (`Err`, the check's
/// own error), because the person or workflow asked for that model. A gate
/// that names none is never refused: the decision model the chat would use
/// judges it, and when the chat has none it may use, its own language model
/// does ([`GateJudgeCheck::ChatModel`], [`judge_gate`]). The app needs only a
/// language model for a gate to work.
pub async fn gate_judge_for(
    service: &DecisionService,
    session: &DecisionSession,
    model: Option<&DecisionModelRef>,
    questions: &IndexMap<String, Question>,
    source: ModelSource,
) -> crate::decision::DecisionResult<GateJudgeCheck> {
    match service
        .check_model_use(session, model, questions, source)
        .await
    {
        Ok(()) => Ok(GateJudgeCheck::DecisionModel),
        Err(why) if model.is_none() => Ok(GateJudgeCheck::ChatModel { why }),
        Err(error) => Err(error),
    }
}

/// Who would judge a gate as it is declared ([`gate_judge_for`]).
#[derive(Debug, Clone, PartialEq)]
pub enum GateJudgeCheck {
    /// A decision model this chat may use answers it.
    DecisionModel,
    /// No decision model can answer it now (`why`), so the chat's own model
    /// judges it when the phase completes, unless one can by then. A
    /// transient `why` (a timeout, a server that is busy or failing) may
    /// clear by then.
    ChatModel { why: crate::decision::DecisionError },
}

/// What asking a gate's decision model came to.
enum DecisionAsk {
    /// The model answered: a verdict, or an answer the gate refuses
    /// (uncalibrated probabilities), with the model's name as a person reads
    /// it.
    Answered { verdict: GateVerdict, model: String },
    /// Nothing was decided.
    Failed(crate::decision::DecisionError),
}

/// Ask the gate's questions of `state` through `service`, as the chat
/// `session` describes (origin `gate`, the phase id and title, the turn).
/// Only a decision model is asked: the decision path of [`judge_gate`], which
/// is what production calls and what falls back to the chat's own model.
#[cfg(test)]
pub(crate) async fn evaluate_gate(
    service: &DecisionService,
    session: &DecisionSession,
    gate: &PhaseGate,
    state: String,
) -> GateVerdict {
    match ask_decision_model(service, session, gate, state).await {
        DecisionAsk::Answered { verdict, .. } => verdict,
        DecisionAsk::Failed(error) => GateVerdict::Error(error.to_string()),
    }
}

async fn ask_decision_model(
    service: &DecisionService,
    session: &DecisionSession,
    gate: &PhaseGate,
    state: String,
) -> DecisionAsk {
    let questions = gate_questions(gate);
    let state = fit_gate_state(service, session, gate.model.as_ref(), &questions, state).await;
    let request = DecisionRequest {
        questions,
        context: DecisionContext::Text(state),
    };
    let decisions = match service
        .decide(Some(session), gate.model.as_ref(), &request)
        .await
    {
        Ok(decisions) => decisions,
        Err(error) => return DecisionAsk::Failed(error),
    };
    let model = match decisions.first() {
        Some(decision) => decision_model_name(service, &decision.model).await,
        None => "the decision model".to_string(),
    };
    if !gate.allow_uncalibrated {
        if let Some(uncalibrated) = decisions.iter().find(|d| !d.calibrated) {
            let verdict = GateVerdict::Error(format!(
                "{} returned uncalibrated probabilities, which a gate refuses. Use a decision \
                 model with its own probabilities, or set allowUncalibrated on the gate to \
                 accept them.",
                uncalibrated.model.label()
            ));
            return DecisionAsk::Answered { verdict, model };
        }
    }
    let answers: Vec<GateAnswer> = gate
        .questions
        .iter()
        .map(|(name, question)| {
            let decision = decisions.iter().find(|d| &d.question == name);
            let p_yes = decision
                .and_then(|d| d.pass_probability)
                .unwrap_or_default();
            GateAnswer {
                question: name.clone(),
                instructions: question.instructions.clone(),
                p_yes,
                passed: decision.is_some_and(|d| d.passed == Some(true)),
            }
        })
        .collect();
    let verdict = if answers.iter().all(|answer| answer.passed) {
        GateVerdict::Passed(answers)
    } else {
        GateVerdict::Failed(answers)
    };
    DecisionAsk::Answered { verdict, model }
}

/// A decision model as a person reads it: its display name from the listing
/// (`joey-1`), else its model id, never `provider/model`.
async fn decision_model_name(service: &DecisionService, model: &DecisionModelRef) -> String {
    service
        .models()
        .await
        .into_iter()
        .find(|entry| entry.answers_to(model))
        .map_or_else(|| model.model.clone(), |entry| entry.display_name)
}

/// The decision model a gate that names none would ask in `session`, as a
/// person reads it: `joey-1 on Baseten`, `tev1:0.8b on Ollama`.
async fn resolved_decision_model_words(
    service: &DecisionService,
    session: &DecisionSession,
) -> Option<String> {
    let entry = service.model_entry_for(Some(session), None).await?;
    let place = match entry.provider.as_str() {
        crate::providers::decision::BASETEN_DECISION_PROVIDER => Some("Baseten"),
        crate::providers::decision::OLLAMA_DECISION_PROVIDER => Some("Ollama"),
        _ => None,
    };
    Some(match place {
        Some(place) => format!("{} on {place}", entry.display_name),
        None => entry.display_name,
    })
}

/// Why a decision model could not be reached, as a clause, when `error` is
/// that kind of failure: the request never reached it (a connection, TLS or
/// address failure), it did not answer in time, or its server failed (HTTP
/// 5xx). `None` for everything else: a refusal, a malformed or unreadable
/// answer, a size or a kind the model cannot take, and a rate limit, all of
/// which stay errors. `name` is the model as a person reads it.
fn unreachable_clause(error: &crate::decision::DecisionError, name: &str) -> Option<String> {
    use crate::decision::DecisionError;
    match error {
        DecisionError::Timeout { seconds, .. } => {
            Some(format!("{name} did not answer within {seconds} s"))
        }
        DecisionError::Backend {
            status: None,
            message,
            ..
        } => Some(
            message
                .trim()
                .trim_end_matches('.')
                .replace('`', "")
                .to_string(),
        ),
        DecisionError::Backend {
            status: Some(status),
            ..
        } if (500..600).contains(status) => {
            Some(format!("{name} failed on its server (HTTP {status})"))
        }
        _ => None,
    }
}

/// How long the chat's own model may take to judge a gate. A local model
/// may need a while to read the state; a stuck call must still end.
const CHAT_GATE_TIMEOUT: Duration = Duration::from_secs(300);

const CHAT_GATE_SYSTEM_PROMPT: &str = "You check whether one phase of a delegated task is \
     done. You read the phase's goal, its hand-off and its subagents' results, and you answer \
     each question yes or no from that text alone. When the text does not show that the answer \
     is yes, the answer is no. For each question give the probability, from 0 to 1, that the \
     answer is yes. Reply with one JSON object and nothing else: no prose and no code fence.";

/// Judge a gate as the chat `session` describes, through a decision model
/// when one may answer it, else through the chat's own language model.
///
/// A gate that names a model asks only that model, through the decision
/// service, and its errors stay errors. A gate that names none asks the decision model the
/// chat would use; the chat's own model `chat_model` judges it instead when
/// the chat has no decision model it may use, or when that model could not be
/// reached ([`unreachable_clause`]). The chat model is the one the chat is
/// bound to, at the chat's own tier, so nothing crosses a privacy boundary:
/// it reads the same screened state a decision model would, which the chat's
/// model may read anyway.
///
/// Returns the verdict and who gave it, when a verdict (passed or not) was
/// reached. An error is never a pass, whoever was asked.
pub(crate) async fn judge_gate(
    service: &DecisionService,
    session: &DecisionSession,
    gate: &PhaseGate,
    state: String,
    chat_model: Option<std::sync::Arc<dyn crate::providers::base::Provider>>,
) -> (GateVerdict, Option<GateJudge>) {
    if gate.model.is_none() {
        let questions = gate_questions(gate);
        let source = ModelSource::of(Some(session), None);
        if let Err(error) = service
            .check_model_use(session, None, &questions, source)
            .await
        {
            return judge_with_chat_model(
                chat_model.as_deref(),
                gate,
                &state,
                "This chat has no decision model it may use",
                &error.to_string(),
            )
            .await;
        }
    }
    let fallback_state = gate.model.is_none().then(|| state.clone());
    match ask_decision_model(service, session, gate, state).await {
        DecisionAsk::Answered { verdict, model } => {
            let judge = (!matches!(verdict, GateVerdict::Error(_))).then_some(GateJudge {
                kind: GateJudgeKind::DecisionModel,
                model,
                reason: None,
            });
            (verdict, judge)
        }
        DecisionAsk::Failed(error) => {
            if let Some(state) = fallback_state {
                let name = resolved_decision_model_words(service, session)
                    .await
                    .unwrap_or_else(|| "The decision model".to_string());
                if let Some(why) = unreachable_clause(&error, &name) {
                    return judge_with_chat_model(
                        chat_model.as_deref(),
                        gate,
                        &state,
                        &why,
                        &error.to_string(),
                    )
                    .await;
                }
            }
            (GateVerdict::Error(error.to_string()), None)
        }
    }
}

/// The chat's own model judging a gate because `why` (a clause: "This chat
/// has no decision model it may use", "joey-1 on Baseten could not be reached
/// (...)"). `decision_error` is what the decision model's attempt said, kept
/// as the error when there is no chat model to ask.
///
/// The request is billed, so every answered one is recorded as a side call
/// ([`SideCall::GateJudge`](crate::session::session_manager::side_usage::SideCall::GateJudge)),
/// an answer that cannot be read included, on the chat the caller scoped with
/// `side_usage::with_side_usage_chat`. Like a decision model's gate, it is
/// counted and never held at a usage limit.
async fn judge_with_chat_model(
    chat_model: Option<&dyn crate::providers::base::Provider>,
    gate: &PhaseGate,
    state: &str,
    why: &str,
    decision_error: &str,
) -> (GateVerdict, Option<GateJudge>) {
    let Some(provider) = chat_model else {
        return (
            GateVerdict::Error(format!(
                "{}. The chat's own model was not available to judge the gate instead",
                decision_error.trim().trim_end_matches('.')
            )),
            None,
        );
    };
    let name = provider.get_model_config().model_name;
    let judged_by = format!("{name}, the chat's model");
    let message =
        crate::conversation::message::Message::user().with_text(chat_gate_prompt(gate, state));
    let reply = tokio::time::timeout(
        CHAT_GATE_TIMEOUT,
        provider.complete(CHAT_GATE_SYSTEM_PROMPT, &[message], &[]),
    )
    .await;
    let text = match reply {
        Ok(Ok((message, usage))) => {
            crate::session::session_manager::side_usage::record_side_call_usage(
                None,
                crate::session::session_manager::side_usage::SideCall::GateJudge,
                provider.get_name(),
                &usage,
            )
            .await;
            message.as_concat_text()
        }
        Ok(Err(error)) => {
            return (
                GateVerdict::Error(format!(
                    "{why}, and {judged_by}, could not judge the gate either: {}",
                    crate::agents::stall::ellipsize(&error.to_string(), 200)
                )),
                None,
            )
        }
        Err(_) => {
            return (
                GateVerdict::Error(format!(
                    "{why}, and {judged_by}, did not judge the gate within {} s",
                    CHAT_GATE_TIMEOUT.as_secs()
                )),
                None,
            )
        }
    };
    let probabilities = match parse_chat_gate_reply(&text, gate) {
        Ok(probabilities) => probabilities,
        Err(detail) => {
            return (
                GateVerdict::Error(format!(
                    "{why}, and {judged_by}, gave an answer that could not be read ({detail})"
                )),
                None,
            )
        }
    };
    let threshold = Threshold::Yes(gate.threshold);
    let answers: Vec<GateAnswer> = gate
        .questions
        .iter()
        .map(|(question, asked)| {
            let p_yes = probabilities.get(question).copied().unwrap_or_default();
            let distribution = IndexMap::from([
                (crate::decision::types::YES.to_string(), p_yes),
                (crate::decision::types::NO.to_string(), 1.0 - p_yes),
            ]);
            GateAnswer {
                question: question.clone(),
                instructions: asked.instructions.clone(),
                p_yes,
                passed: threshold.passes(&distribution),
            }
        })
        .collect();
    let verdict = if answers.iter().all(|answer| answer.passed) {
        GateVerdict::Passed(answers)
    } else {
        GateVerdict::Failed(answers)
    };
    let judge = GateJudge {
        kind: GateJudgeKind::ChatModel,
        model: name,
        reason: Some(format!("{why}, so {judged_by}, judged the gate.")),
    };
    (verdict, Some(judge))
}

/// The one message the chat's model reads: the gate's state, its questions
/// and the exact reply shape.
fn chat_gate_prompt(gate: &PhaseGate, state: &str) -> String {
    let questions: String = gate
        .questions
        .iter()
        .map(|(name, question)| format!("\n- {name}: {}", question.instructions.trim()))
        .collect();
    let example = gate
        .questions
        .keys()
        .next()
        .map(|name| serde_json::json!({ name: { "answer": "yes", "p_yes": 0.9 } }).to_string())
        .unwrap_or_default();
    format!(
        "{state}\n\nQuestions:{questions}\n\nReply with only a JSON object that has one entry \
         for each question name above, each {{\"answer\": \"yes\" or \"no\", \"p_yes\": the \
         probability from 0 to 1 that the answer is yes}}. For example: {example}"
    )
}

/// The chat model's reply read strictly: one JSON object (a code fence
/// around it is tolerated, prose is not), one entry per gate question and no
/// other, each an `answer` of yes or no with a `p_yes` from 0 to 1 that agrees
/// with it. `Err` says what was wrong, in a clause; it never quotes the reply.
fn parse_chat_gate_reply(text: &str, gate: &PhaseGate) -> Result<IndexMap<String, f64>, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ChatGateAnswer {
        answer: String,
        p_yes: f64,
    }
    let mut body = text.trim();
    if let Some(fenced) = body.strip_prefix("```") {
        let after_tag = fenced.split_once('\n').map_or("", |(_, rest)| rest);
        body = after_tag.trim_end().trim_end_matches("```").trim();
    }
    let parsed: IndexMap<String, ChatGateAnswer> = serde_json::from_str(body)
        .map_err(|_| "it was not one JSON object of answers".to_string())?;
    if let Some(extra) = parsed
        .keys()
        .find(|name| !gate.questions.contains_key(*name))
    {
        return Err(format!(
            "it answered a question the gate does not ask, {}",
            truncate_chars(extra, 64)
        ));
    }
    let mut probabilities = IndexMap::new();
    for name in gate.questions.keys() {
        let answer = parsed
            .get(name)
            .ok_or_else(|| format!("it did not answer {name}"))?;
        let p_yes = answer.p_yes;
        if !p_yes.is_finite() || !(0.0..=1.0).contains(&p_yes) {
            return Err(format!("its P(yes) for {name} was not between 0 and 1"));
        }
        let slack = crate::decision::types::THRESHOLD_EPSILON;
        match answer.answer.trim().to_ascii_lowercase().as_str() {
            "yes" if p_yes + slack >= 0.5 => {}
            "no" if p_yes <= 0.5 + slack => {}
            "yes" | "no" => {
                return Err(format!(
                    "its answer to {name} disagreed with its P(yes) of {p_yes:.2}"
                ))
            }
            _ => return Err(format!("its answer to {name} was not yes or no")),
        }
        probabilities.insert(name.clone(), p_yes);
    }
    Ok(probabilities)
}

/// Keep who gave a verdict with it: `judge` on the gate of `phase_id`, after
/// [`apply_gate_verdict`] wrote the verdict it gave. Nothing for an error,
/// which leaves the last verdict and its judge as they were.
pub(crate) fn record_gate_judge(plan: &mut OrchestrationPlan, phase_id: &str, judge: &GateJudge) {
    if let Some(state) = plan
        .phase_mut(phase_id)
        .and_then(|phase| phase.gate_state.as_mut())
    {
        state.last_judge = Some(judge.clone());
    }
}

fn min_p(answers: &[GateAnswer]) -> Option<f64> {
    answers
        .iter()
        .map(|answer| answer.p_yes)
        .fold(None, |acc: Option<f64>, p| {
            Some(acc.map_or(p, |a| a.min(p)))
        })
}

fn answers_words(answers: &[GateAnswer]) -> String {
    answers
        .iter()
        .map(|answer| format!("{} P(yes) {:.2}", answer.question, answer.p_yes))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Write one evaluation's verdict. Returns the plan, the sentence for the
/// model, and whether the phase closed (the caller then checks for racing
/// spawns, as it does for an ungated completion).
///
/// Refused, with nothing written, when the gate changed or the phase closed
/// while the gate was being evaluated, or (on a pass) when an earlier gate
/// now holds the phase or the barrier no longer holds.
pub(crate) fn apply_gate_verdict(
    plan: Option<OrchestrationPlan>,
    phase_id: &str,
    evaluated: &PhaseGate,
    verdict: &GateVerdict,
    summary: Option<&str>,
    agents: &[PhaseAgentState],
    now: DateTime<Utc>,
) -> Result<(OrchestrationPlan, String, bool), String> {
    let plan = plan.ok_or_else(|| NO_PLAN.to_string())?;
    {
        let unknown = plan.unknown_phase(phase_id);
        let phase = plan.phase(phase_id).ok_or(unknown)?;
        if phase.gate.as_ref() != Some(evaluated) {
            return Err(format!(
                "Phase \"{phase_id}\"'s gate changed while it was being evaluated. Nothing was \
                 changed; complete the phase again."
            ));
        }
        if phase.status.is_closed() {
            return Err(format!(
                "Phase \"{phase_id}\" was {} while its gate was being evaluated. Nothing was \
                 changed.",
                phase.status.as_str()
            ));
        }
    }
    let threshold = evaluated.threshold;
    let max_attempts = evaluated.max_attempts;

    if let GateVerdict::Passed(answers) = verdict {
        // A person may have set a gate on an earlier phase while this one
        // was evaluated: a held phase does not close, even on a pass.
        if let Some(holder) = hold_of(&plan, phase_id) {
            return Err(format!(
                "Phase \"{phase_id}\" now waits for phase \"{holder}\"'s gate, set while its own \
                 gate was evaluated. The verdict was not recorded; complete it again once \
                 \"{holder}\" is through."
            ));
        }
        let (mut plan, note) =
            close_phase(plan, phase_id, PhaseStatus::Completed, summary, agents, now)?;
        let phase = plan.phase_mut(phase_id).expect("completed above");
        let state = phase.gate_state.get_or_insert_with(GateState::default);
        state.attempts += 1;
        state.consecutive_errors = 0;
        state.last_passed = Some(true);
        state.last_min_p = min_p(answers);
        state.last_error = None;
        state.last_at = Some(now);
        // The gate itself closed the phase, so no decision of the person's
        // stands any more.
        state.person = None;
        let note = format!(
            "{note} Its gate passed on attempt {} of {max_attempts}: {} (threshold {threshold:.2}).",
            state.attempts,
            answers_words(answers)
        );
        return Ok((plan, note, true));
    }

    let summary = optional_text(summary, MAX_HANDOFF_CHARS, "The hand-off summary")?;
    let mut plan = plan;
    let phase = plan.phase_mut(phase_id).expect("checked above");
    if let Some(summary) = summary {
        phase.handoff = Some(PhaseHandoff {
            summary,
            source: HandoffSource::Parent,
            written_at: now,
        });
    }
    let note = match verdict {
        GateVerdict::Passed(_) => unreachable!("handled above"),
        GateVerdict::Failed(answers) => record_gate_failure(phase, evaluated, answers, now),
        GateVerdict::Error(message) => record_gate_error(phase, message, now),
    };
    Ok((plan, note, false))
}

/// A failed verdict: one attempt used. The phase stays in progress with a
/// fix-it instruction, or is blocked once the attempts are used up.
fn record_gate_failure(
    phase: &mut PlanPhase,
    gate: &PhaseGate,
    answers: &[GateAnswer],
    now: DateTime<Utc>,
) -> String {
    let (threshold, max_attempts) = (gate.threshold, gate.max_attempts);
    let state = phase.gate_state.get_or_insert_with(GateState::default);
    state.last_at = Some(now);
    state.attempts += 1;
    state.consecutive_errors = 0;
    state.last_passed = Some(false);
    state.last_min_p = min_p(answers);
    state.last_error = None;
    let attempts = state.attempts;
    let failing: String = answers
        .iter()
        .filter(|answer| !answer.passed)
        .map(|answer| {
            format!(
                "\n- {}: P(yes) {:.2}, below {threshold:.2}. Asked: \"{}\"",
                answer.question, answer.p_yes, answer.instructions
            )
        })
        .collect();
    let phase_id = phase.id.clone();
    if attempts >= max_attempts {
        phase.status = PhaseStatus::Blocked;
        phase.finished_at = Some(now);
        return format!(
            "Phase \"{phase_id}\" is blocked: its gate failed all {max_attempts} attempts. The \
             questions that fell short:{failing}\nTell the person what the gate asked and what \
             the results lacked. {ONLY_THE_PERSON}"
        );
    }
    if phase.status != PhaseStatus::InProgress {
        phase.status = PhaseStatus::InProgress;
        phase.started_at.get_or_insert(now);
        phase.finished_at = None;
    }
    format!(
        "Phase \"{phase_id}\" was NOT completed: its gate failed on attempt {attempts} of \
         {max_attempts}. Every question must reach P(yes) >= {threshold:.2}, and these fell \
         short:{failing}\nFix what they point at: spawn subagents into the phase to do the \
         missing work, or write a hand-off that states the evidence with action:\"handoff\". \
         Then complete the phase again. It stays in progress."
    )
}

/// An evaluation that decided nothing: no attempt used; the second in a row
/// blocks the phase.
fn record_gate_error(phase: &mut PlanPhase, message: &str, now: DateTime<Utc>) -> String {
    let state = phase.gate_state.get_or_insert_with(GateState::default);
    state.last_at = Some(now);
    state.consecutive_errors += 1;
    state.last_error = Some(message.to_string());
    let blocked = state.consecutive_errors >= GATE_ERRORS_TO_BLOCK;
    let phase_id = phase.id.clone();
    if blocked {
        phase.status = PhaseStatus::Blocked;
        phase.finished_at = Some(now);
        return format!(
            "Phase \"{phase_id}\" is blocked: its gate could not be evaluated \
             {GATE_ERRORS_TO_BLOCK} times in a row ({message}). Nothing was decided. Tell the \
             person what the gate asked and that it could not be answered. Once the model \
             that judges it can be reached, you can complete the phase again to ask it; \
             otherwise only the person can go on: they can give the gate its attempts again, \
             change or remove it, or pass or skip the phase, in the app or with `biorouter \
             session gates`."
        );
    }
    format!(
        "Phase \"{phase_id}\" was NOT completed: its gate could not be evaluated ({message}). \
         Nothing was decided and no attempt was used. Complete the phase again to retry; a \
         second failure in a row blocks it."
    )
}

// ---------------------------------------------------------------------------
// The person's gate controls. Only the person sets, replaces, removes, passes,
// skips or retries a gate; the HTTP routes that call these require the
// user-action proof, and the agent's tool has no such action. Every write is
// one per-key transaction on the plan, like the agent's.
// ---------------------------------------------------------------------------

const NO_PLAN_TO_GATE: &str = "This chat has no orchestration plan, so it has no phases to gate.";

/// Why a person's gate write changed nothing. Each variant's text is the
/// sentence the person reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateWriteError {
    /// The chat has no plan.
    NoPlan,
    /// The phase is not in the plan; the sentence names the plan's phases.
    NoPhase(String),
    /// The phase is completed or skipped.
    Closed(String),
    /// The phase has no gate to remove or decide.
    NoGate(String),
    /// The gate given is not one a phase can carry.
    Invalid(String),
    /// The plan could not be read or written.
    Unreadable(String),
}

impl GateWriteError {
    pub fn message(&self) -> String {
        match self {
            Self::NoPlan => NO_PLAN_TO_GATE.to_string(),
            Self::NoPhase(sentence)
            | Self::Closed(sentence)
            | Self::NoGate(sentence)
            | Self::Invalid(sentence)
            | Self::Unreadable(sentence) => sentence.clone(),
        }
    }
}

impl std::fmt::Display for GateWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for GateWriteError {}

/// What a person's gate write did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PersonGateOutcome {
    /// False when a pass or skip was recorded but could not close the phase
    /// yet: it stands until the agent completes the phase.
    pub applied: bool,
    /// The sentence the person reads.
    pub message: String,
}

impl PersonGateOutcome {
    fn applied(message: String) -> Self {
        Self {
            applied: true,
            message,
        }
    }
}

/// The index of the OPEN phase `phase_id`, the only kind a gate holds.
fn person_target(plan: &OrchestrationPlan, phase_id: &str) -> Result<usize, GateWriteError> {
    let index = plan.index_of(phase_id).ok_or_else(|| {
        GateWriteError::NoPhase(format!(
            "{} Nothing was changed.",
            plan.unknown_phase(phase_id)
        ))
    })?;
    let phase = &plan.phases[index];
    if phase.status.is_closed() {
        return Err(GateWriteError::Closed(format!(
            "Phase \"{phase_id}\" is already {}; a gate holds only an open phase. Nothing was \
             changed.",
            phase.status.as_str()
        )));
    }
    Ok(index)
}

fn no_gate(phase_id: &str) -> GateWriteError {
    GateWriteError::NoGate(format!(
        "Phase \"{phase_id}\" has no gate. Nothing was changed."
    ))
}

/// A phase its gate blocked goes back to in progress, so the agent can work
/// on it again under the person's new terms.
fn reopen_if_gate_blocked(
    phase: &mut PlanPhase,
    was_gate_blocked: bool,
    now: Option<DateTime<Utc>>,
) {
    if was_gate_blocked && phase.status == PhaseStatus::Blocked {
        phase.status = PhaseStatus::InProgress;
        if let Some(now) = now {
            phase.started_at.get_or_insert(now);
        }
        phase.finished_at = None;
    }
}

/// What now becomes of the phases after the one at `index`: the next open
/// one is free, or another gate still holds it.
fn after_note(plan: &OrchestrationPlan, index: usize) -> String {
    let Some(next) =
        (index + 1..plan.phases.len()).find(|later| !plan.phases[*later].status.is_closed())
    else {
        return String::new();
    };
    match plan.holder_index(next) {
        Some(holder) => format!(
            " The phases after it still wait for phase \"{}\"'s gate.",
            plan.phases[holder].id
        ),
        None => " The phases after it can go on.".to_string(),
    }
}

/// The person sets the gate of an open phase, replacing any gate it had
/// (the agent's, the workflow's or their own): the gate is owned by the
/// person and its record starts over, and a phase its old gate blocked goes
/// back to in progress. The gate is checked again as [`validate_person_gate`]
/// checks it.
pub fn apply_person_set_gate(
    plan: Option<OrchestrationPlan>,
    phase_id: &str,
    gate: PhaseGate,
    now: DateTime<Utc>,
) -> Result<(OrchestrationPlan, PersonGateOutcome), GateWriteError> {
    let mut plan = plan.ok_or(GateWriteError::NoPlan)?;
    let phase_id = phase_id.trim();
    let index = person_target(&plan, phase_id)?;
    let questions: IndexMap<String, String> = gate
        .questions
        .iter()
        .map(|(name, question)| (name.clone(), question.instructions.clone()))
        .collect();
    let gate = build_gate(
        phase_id,
        &questions,
        Some(gate.threshold),
        Some(gate.max_attempts),
        gate.model,
        Some(gate.allow_uncalibrated),
        GateOwner::Person,
    )
    .map_err(GateWriteError::Invalid)?;

    let phase = &mut plan.phases[index];
    let replaced = match phase.gate.as_ref().map(|gate| gate.owner) {
        None => "",
        Some(GateOwner::Agent) => {
            " It replaces the gate the agent set, and its attempts start over."
        }
        Some(GateOwner::Workflow) => {
            " It replaces the gate the workflow set, and its attempts start over."
        }
        Some(GateOwner::Person) => " It replaces your earlier gate, and its attempts start over.",
    };
    let was_gate_blocked = gate_blocked(phase);
    let questions = gate.questions.len();
    let mut message = format!(
        "Phase \"{phase_id}\" now has your gate: {questions} question{}, each needing P(yes) of \
         at least {:.2}, with {} attempt{}.{replaced} The agent cannot change, remove or skip \
         it, and the phases after it wait until it passes or you pass or skip the phase.",
        if questions == 1 { "" } else { "s" },
        gate.threshold,
        gate.max_attempts,
        if gate.max_attempts == 1 { "" } else { "s" },
    );
    phase.gate = Some(gate);
    phase.gate_state = None;
    reopen_if_gate_blocked(phase, was_gate_blocked, Some(now));
    if was_gate_blocked {
        message.push_str(" It is back in progress.");
    }
    Ok((plan, PersonGateOutcome::applied(message)))
}

/// The person removes the gate of an open phase: its record goes with it, a
/// phase the gate blocked goes back to in progress, and the phases after it
/// are no longer held by it.
pub fn apply_person_remove_gate(
    plan: Option<OrchestrationPlan>,
    phase_id: &str,
) -> Result<(OrchestrationPlan, PersonGateOutcome), GateWriteError> {
    let mut plan = plan.ok_or(GateWriteError::NoPlan)?;
    let phase_id = phase_id.trim();
    let index = person_target(&plan, phase_id)?;
    let phase = &mut plan.phases[index];
    if phase.gate.is_none() {
        return Err(no_gate(phase_id));
    }
    let was_gate_blocked = gate_blocked(phase);
    phase.gate = None;
    phase.gate_state = None;
    reopen_if_gate_blocked(phase, was_gate_blocked, None);
    let mut message =
        format!("Phase \"{phase_id}\" has no gate now: the agent can complete it without one.");
    if was_gate_blocked {
        message.push_str(" It is back in progress.");
    }
    message.push_str(&after_note(&plan, index));
    Ok((plan, PersonGateOutcome::applied(message)))
}

/// The person's pass, skip or retry of a gated open phase.
///
/// - Retry gives the gate its attempts again: its count and errors start
///   over, a standing pass or skip is dropped, and a phase the gate blocked
///   goes back to in progress.
/// - Pass or skip is recorded, then the phase is closed (completed or
///   skipped) without its gate when the barrier allows: no subagent of the
///   phase still queued, running or cancelling. An earlier gate does not hold
///   the person. When the barrier refuses, the decision STANDS
///   (`applied: false`): the agent's next `complete` with the matching status
///   applies it without asking the gate.
///
/// `agents` are the phase's child states, read before the write as
/// `complete` reads them.
pub fn apply_person_gate_decision(
    plan: Option<OrchestrationPlan>,
    phase_id: &str,
    choice: PersonGateChoice,
    agents: &[PhaseAgentState],
    now: DateTime<Utc>,
) -> Result<(OrchestrationPlan, PersonGateOutcome), GateWriteError> {
    person_decision(plan, phase_id, choice, agents, now, true)
}

/// [`apply_person_gate_decision`]; `may_close` is false while a spawn into
/// the phase is in flight, so a pass or skip only stands.
fn person_decision(
    plan: Option<OrchestrationPlan>,
    phase_id: &str,
    choice: PersonGateChoice,
    agents: &[PhaseAgentState],
    now: DateTime<Utc>,
    may_close: bool,
) -> Result<(OrchestrationPlan, PersonGateOutcome), GateWriteError> {
    let mut plan = plan.ok_or(GateWriteError::NoPlan)?;
    let phase_id = phase_id.trim();
    let index = person_target(&plan, phase_id)?;
    let Some(max_attempts) = plan.phases[index]
        .gate
        .as_ref()
        .map(|gate| gate.max_attempts)
    else {
        return Err(no_gate(phase_id));
    };
    let (action, status) = match choice {
        PersonGateChoice::Retry => {
            let phase = &mut plan.phases[index];
            let was_gate_blocked = gate_blocked(phase);
            let state = phase.gate_state.get_or_insert_with(GateState::default);
            state.attempts = 0;
            state.consecutive_errors = 0;
            state.last_passed = None;
            state.last_min_p = None;
            state.last_error = None;
            state.person = None;
            state.retries_granted += 1;
            reopen_if_gate_blocked(phase, was_gate_blocked, Some(now));
            let message = format!(
                "Phase \"{phase_id}\"'s gate has its {max_attempts} attempt{} again. The agent \
                 can complete the phase once more to ask it.",
                if max_attempts == 1 { "" } else { "s" }
            );
            return Ok((plan, PersonGateOutcome::applied(message)));
        }
        PersonGateChoice::Pass => (PersonGateAction::Pass, PhaseStatus::Completed),
        PersonGateChoice::Skip => (PersonGateAction::Skip, PhaseStatus::Skipped),
    };
    plan.phases[index]
        .gate_state
        .get_or_insert_with(GateState::default)
        .person = Some(PersonGateDecision {
        decision: action,
        at: now,
    });
    if may_close {
        let closed = close_by_person(plan.clone(), phase_id, action, |plan| {
            close_phase(plan, phase_id, status, None, agents, now)
        });
        if let Ok((closed, _)) = closed {
            let message = match action {
                PersonGateAction::Pass => {
                    format!("Phase \"{phase_id}\" is completed: you passed its gate.")
                }
                PersonGateAction::Skip => format!(
                    "Phase \"{phase_id}\" is skipped: you skipped it, so its gate was not asked."
                ),
            };
            let message = format!("{message}{}", after_note(&closed, index));
            return Ok((closed, PersonGateOutcome::applied(message)));
        }
    }

    // The barrier refused: the decision stands for the agent to apply.
    let phase = &plan.phases[index];
    let checked: HashSet<&str> = agents
        .iter()
        .map(|agent| agent.child_session_id.as_str())
        .collect();
    let working = agents
        .iter()
        .filter(|agent| agent.status.is_active())
        .count()
        + phase
            .agents
            .iter()
            .filter(|agent| !checked.contains(agent.child_session_id.as_str()))
            .count();
    let (decision, outcome) = match action {
        PersonGateAction::Pass => ("pass", "completes"),
        PersonGateAction::Skip => ("skip", "is skipped"),
    };
    let once = match working {
        0 => "once the subagent being started in it finishes".to_string(),
        1 => "once its 1 working subagent finishes".to_string(),
        n => format!("once its {n} working subagents finish"),
    };
    let message = format!(
        "Your {decision} stands: phase \"{phase_id}\" {outcome} without its gate {once} and the \
         agent completes it."
    );
    Ok((
        plan,
        PersonGateOutcome {
            applied: false,
            message,
        },
    ))
}

/// Run a person's write on the plan of `session_id` in one transaction,
/// keeping the variant of a refusal.
async fn write_as_person<F>(
    session_manager: &SessionManager,
    session_id: &str,
    write: F,
) -> Result<PersonGateOutcome, GateWriteError>
where
    F: FnOnce(
            Option<OrchestrationPlan>,
        ) -> Result<(OrchestrationPlan, PersonGateOutcome), GateWriteError>
        + Send,
{
    let mut refused: Option<GateWriteError> = None;
    let slot = &mut refused;
    let written = mutate_plan(session_manager, session_id, move |current| {
        write(current).map_err(|refusal| {
            let sentence = refusal.message();
            *slot = Some(refusal);
            sentence
        })
    })
    .await;
    match written {
        Ok(outcome) => Ok(outcome),
        Err(e) => Err(refused.unwrap_or_else(|| {
            GateWriteError::Unreadable(format!("Could not change this chat's phase gates: {e}."))
        })),
    }
}

/// [`apply_person_set_gate`] on the stored plan of `session_id`. The gate
/// is the person's: build it with [`validate_person_gate`], and check that
/// the chat may use its model before calling this.
pub async fn person_set_gate(
    session_manager: &SessionManager,
    session_id: &str,
    phase_id: &str,
    gate: PhaseGate,
) -> Result<PersonGateOutcome, GateWriteError> {
    let now = Utc::now();
    let phase_id = phase_id.trim().to_string();
    write_as_person(session_manager, session_id, move |current| {
        apply_person_set_gate(current, &phase_id, gate, now)
    })
    .await
}

/// [`apply_person_remove_gate`] on the stored plan of `session_id`.
pub async fn person_remove_gate(
    session_manager: &SessionManager,
    session_id: &str,
    phase_id: &str,
) -> Result<PersonGateOutcome, GateWriteError> {
    let phase_id = phase_id.trim().to_string();
    write_as_person(session_manager, session_id, move |current| {
        apply_person_remove_gate(current, &phase_id)
    })
    .await
}

/// [`apply_person_gate_decision`] on the stored plan of `session_id`, with
/// the barrier `complete` uses: the phase's children are read before the
/// write, and a spawn into the phase in flight (or begun while this ran)
/// keeps a pass or skip standing rather than closing the phase over it.
pub async fn person_gate_decision(
    session_manager: &SessionManager,
    session_id: &str,
    phase_id: &str,
    choice: PersonGateChoice,
) -> Result<PersonGateOutcome, GateWriteError> {
    let now = Utc::now();
    let phase_id = phase_id.trim().to_string();
    let spawns_before = phase_spawn_mark(session_id, &phase_id);
    let states = if choice == PersonGateChoice::Retry {
        Vec::new()
    } else {
        let stored = load_plan(session_manager, session_id).await.map_err(|e| {
            GateWriteError::Unreadable(format!("Could not change this chat's phase gates: {e}."))
        })?;
        let agents = stored
            .as_ref()
            .and_then(|plan| plan.phase(&phase_id))
            .map(|phase| phase.agents.clone())
            .unwrap_or_default();
        phase_agent_states(session_manager, session_id, &agents).await
    };
    write_as_person(session_manager, session_id, move |current| {
        let may_close = refuse_racing_spawns(
            &phase_id,
            spawns_before,
            phase_spawn_mark(session_id, &phase_id),
        )
        .is_ok();
        person_decision(current, &phase_id, choice, &states, now, may_close)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::SessionClassification;
    use crate::session::session_manager::SessionType;
    use std::sync::Arc;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-04T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn input(id: &str, kind: &str, depends_on: &[&str]) -> PlanPhaseInput {
        PlanPhaseInput {
            id: id.to_string(),
            title: format!("{id} title"),
            kind: Some(kind.to_string()),
            goal: None,
            depends_on: depends_on.iter().map(|d| d.to_string()).collect(),
        }
    }

    fn three_phase_plan() -> OrchestrationPlan {
        apply_plan(
            None,
            Some("Ship the parser"),
            Some("A parser with tests"),
            &[
                input("research", "research", &[]),
                input("build", "implement", &["research"]),
                input("verify", "verify", &[]),
            ],
        )
        .unwrap()
    }

    fn row(id: &str, status: SubagentRunStatus, quiet: Option<u64>) -> SubagentRow {
        SubagentRow {
            session_id: id.to_string(),
            name: format!("{id} name"),
            role: None,
            phase_id: None,
            status,
            loop_phase: None,
            quiet_seconds: quiet,
            created_at: now(),
            updated_at: now(),
            ended_at: None,
            elapsed_seconds: None,
            provider_name: None,
            model_name: None,
            extensions: Vec::new(),
            skills: Vec::new(),
            knowledge_bases: Vec::new(),
            privacy_tier: SessionClassification::Public,
            total_tokens: None,
            message_count: 0,
            tool_call_count: 0,
            result_line: None,
        }
    }

    fn state(id: &str, status: SubagentRunStatus, line: Option<&str>) -> PhaseAgentState {
        PhaseAgentState {
            child_session_id: id.to_string(),
            label: format!("{id} role"),
            status,
            result_line: line.map(str::to_string),
            withheld: false,
        }
    }

    fn with_agents(mut plan: OrchestrationPlan, phase: &str, ids: &[&str]) -> OrchestrationPlan {
        for id in ids {
            plan = apply_record_spawn(Some(plan), phase, id, Some("worker"), now()).unwrap();
        }
        plan
    }

    #[test]
    fn plan_validation_refuses_each_bad_shape_with_a_reason() {
        let long_title = "t".repeat(81);
        let long_id = "x".repeat(41);
        let cases: Vec<(Option<&str>, Vec<PlanPhaseInput>, &str)> = vec![
            (None, vec![input("a", "plan", &[])], "needs a title"),
            (Some("t"), vec![], "at least one phase"),
            (
                Some("t"),
                (0..13)
                    .map(|i| input(&format!("p{i}"), "other", &[]))
                    .collect(),
                "at most 12 phases",
            ),
            (Some("t"), vec![input("Bad Id", "plan", &[])], "not a slug"),
            (Some("t"), vec![input(&long_id, "plan", &[])], "not a slug"),
            (
                Some("t"),
                vec![input("a", "plan", &[]), input("a", "plan", &[])],
                "appears twice",
            ),
            (
                Some("t"),
                vec![input("a", "plan", &["b"]), input("b", "plan", &[])],
                "not an earlier phase",
            ),
            (
                Some("t"),
                vec![input("a", "plan", &["a"])],
                "cannot depend on itself",
            ),
            (
                Some(long_title.as_str()),
                vec![input("a", "plan", &[])],
                "limit is 80",
            ),
        ];
        for (title, phases, expected) in cases {
            let err = apply_plan(None, title, None, &phases).unwrap_err();
            assert!(err.contains(expected), "expected {expected:?} in {err:?}");
        }

        let mut long_goal = input("a", "plan", &[]);
        long_goal.goal = Some("g".repeat(501));
        let err = apply_plan(None, Some("t"), None, &[long_goal]).unwrap_err();
        assert!(err.contains("limit is 500"), "{err}");
    }

    #[test]
    fn plan_accepts_slugs_kinds_and_earlier_dependencies() {
        let plan = three_phase_plan();
        assert_eq!(plan.title, "Ship the parser");
        assert_eq!(plan.goal.as_deref(), Some("A parser with tests"));
        let kinds: Vec<PhaseKind> = plan.phases.iter().map(|p| p.kind).collect();
        assert_eq!(
            kinds,
            vec![
                PhaseKind::Research,
                PhaseKind::Implementation,
                PhaseKind::Verification
            ]
        );
        assert!(plan.phases.iter().all(|p| p.status == PhaseStatus::Pending));
        assert_eq!(plan.phases[1].depends_on, vec!["research".to_string()]);
        assert_eq!(PhaseKind::parse_lenient("brainstorm"), PhaseKind::Other);
        assert_eq!(PhaseKind::parse_lenient("Meeting"), PhaseKind::Meeting);
    }

    #[test]
    fn revising_keeps_agents_status_and_handoff_of_phases_whose_ids_persist() {
        let mut plan = with_agents(three_phase_plan(), "research", &["c1", "c2"]);
        plan = apply_handoff(Some(plan), "research", "found three papers", now()).unwrap();

        let revised = apply_plan(
            Some(plan.clone()),
            None,
            None,
            &[
                PlanPhaseInput {
                    title: "Read the literature".into(),
                    ..input("research", "research", &[])
                },
                input("meet", "meeting", &["research"]),
                input("build", "implementation", &["meet"]),
            ],
        )
        .unwrap();
        assert_eq!(revised.title, "Ship the parser", "title kept when omitted");
        let research = revised.phase("research").unwrap();
        assert_eq!(research.title, "Read the literature");
        assert_eq!(research.status, PhaseStatus::InProgress);
        assert_eq!(research.agents.len(), 2);
        assert_eq!(
            research.handoff.as_ref().unwrap().summary,
            "found three papers"
        );
        assert_eq!(revised.phase("build").unwrap().depends_on, vec!["meet"]);
        assert!(
            revised.phase("verify").is_none(),
            "an agentless phase may go"
        );

        // Removing a phase that already ran subagents is refused, by name.
        let err =
            apply_plan(Some(plan), None, None, &[input("build", "implement", &[])]).unwrap_err();
        assert!(err.contains("removes research"), "{err}");
        assert!(err.contains("skipped"), "{err}");
    }

    #[test]
    fn start_moves_pending_to_in_progress_and_refuses_closed_phases() {
        let (plan, note) = apply_start(Some(three_phase_plan()), "build", now()).unwrap();
        let build = plan.phase("build").unwrap();
        assert_eq!(build.status, PhaseStatus::InProgress);
        assert_eq!(build.started_at, Some(now()));
        assert!(note.contains("depends on research"), "{note}");

        let (plan, note) = apply_start(Some(plan), "build", now()).unwrap();
        assert!(note.contains("already in progress"), "{note}");

        let (plan, _) = apply_complete(
            Some(plan),
            "build",
            PhaseStatus::Completed,
            None,
            &[],
            now(),
        )
        .unwrap();
        let err = apply_start(Some(plan.clone()), "build", now()).unwrap_err();
        assert!(err.contains("already completed"), "{err}");
        let err = apply_start(Some(plan), "nope", now()).unwrap_err();
        assert!(err.contains("research, build, verify"), "{err}");
        let err = apply_start(None, "build", now()).unwrap_err();
        assert!(err.contains("no orchestration plan"), "{err}");
    }

    #[test]
    fn complete_is_a_barrier_while_any_child_is_active() {
        let plan = with_agents(three_phase_plan(), "research", &["c1", "c2"]);
        let agents = [
            state("c1", SubagentRunStatus::Completed, Some("done")),
            state("c2", SubagentRunStatus::Running, None),
        ];
        let err = apply_complete(
            Some(plan.clone()),
            "research",
            PhaseStatus::Completed,
            None,
            &agents,
            now(),
        )
        .unwrap_err();
        assert!(err.contains("cannot be completed yet"), "{err}");
        assert!(err.contains("c2"), "{err}");
        assert!(err.contains("\"mode\": \"all\""), "{err}");
        assert!(
            !err.contains("c1 ("),
            "a finished child is not named: {err}"
        );

        // Every active state holds the barrier, including a skip.
        for active in [
            SubagentRunStatus::Queued,
            SubagentRunStatus::Running,
            SubagentRunStatus::Cancelling,
        ] {
            let agents = [
                state("c1", SubagentRunStatus::Completed, None),
                state("c2", active, None),
            ];
            assert!(apply_complete(
                Some(plan.clone()),
                "research",
                PhaseStatus::Skipped,
                None,
                &agents,
                now()
            )
            .is_err());
        }

        // A child recorded after the states were gathered was never checked.
        let err = apply_complete(
            Some(plan.clone()),
            "research",
            PhaseStatus::Completed,
            None,
            &agents[..1],
            now(),
        )
        .unwrap_err();
        assert!(err.contains("while this call ran (c2)"), "{err}");

        // Once nothing is active it completes.
        let settled = [
            state("c1", SubagentRunStatus::Completed, Some("done")),
            state("c2", SubagentRunStatus::Failed, Some("tool failed")),
        ];
        let (plan, note) = apply_complete(
            Some(plan),
            "research",
            PhaseStatus::Completed,
            None,
            &settled,
            now(),
        )
        .unwrap();
        let research = plan.phase("research").unwrap();
        assert_eq!(research.status, PhaseStatus::Completed);
        assert_eq!(research.finished_at, Some(now()));
        assert!(note.contains("composed"), "{note}");

        let err = apply_complete(
            Some(plan),
            "research",
            PhaseStatus::Completed,
            None,
            &settled,
            now(),
        )
        .unwrap_err();
        assert!(err.contains("already completed"), "{err}");
    }

    #[test]
    fn complete_composes_a_handoff_only_when_none_was_written() {
        let plan = with_agents(three_phase_plan(), "research", &["c1", "c2"]);
        let settled = [
            state(
                "c1",
                SubagentRunStatus::Completed,
                Some("Found 3 papers\nmore"),
            ),
            state("c2", SubagentRunStatus::Incomplete, None),
        ];
        let (auto, _) = apply_complete(
            Some(plan.clone()),
            "research",
            PhaseStatus::Completed,
            None,
            &settled,
            now(),
        )
        .unwrap();
        let handoff = auto.phase("research").unwrap().handoff.clone().unwrap();
        assert_eq!(handoff.source, HandoffSource::Auto);
        assert!(
            handoff.summary.contains("2 subagents"),
            "{}",
            handoff.summary
        );
        assert!(
            handoff
                .summary
                .contains("c1 role [completed] (c1): Found 3 papers"),
            "{}",
            handoff.summary
        );
        assert!(handoff.summary.contains("c2 role [incomplete]"));
        assert!(handoff.summary.contains("no result line"));

        // A summary on complete wins and is the parent's.
        let (given, _) = apply_complete(
            Some(plan.clone()),
            "research",
            PhaseStatus::Completed,
            Some("Use paper B"),
            &settled,
            now(),
        )
        .unwrap();
        let handoff = given.phase("research").unwrap().handoff.clone().unwrap();
        assert_eq!(handoff.source, HandoffSource::Parent);
        assert_eq!(handoff.summary, "Use paper B");

        // An earlier written hand-off is kept.
        let written = apply_handoff(Some(plan.clone()), "research", "mine", now()).unwrap();
        let (kept, _) = apply_complete(
            Some(written),
            "research",
            PhaseStatus::Completed,
            None,
            &settled,
            now(),
        )
        .unwrap();
        assert_eq!(
            kept.phase("research")
                .unwrap()
                .handoff
                .as_ref()
                .unwrap()
                .summary,
            "mine"
        );

        // Blocked or skipped never composes one.
        let (blocked, _) = apply_complete(
            Some(plan),
            "research",
            PhaseStatus::Blocked,
            None,
            &settled,
            now(),
        )
        .unwrap();
        assert!(blocked.phase("research").unwrap().handoff.is_none());
        assert!(compose_auto_handoff(&[]).is_none());

        // The composed text respects the hand-off cap.
        let many: Vec<PhaseAgentState> = (0..40)
            .map(|i| {
                state(
                    &format!("c{i}"),
                    SubagentRunStatus::Completed,
                    Some(&"r".repeat(300)),
                )
            })
            .collect();
        let composed = compose_auto_handoff(&many).unwrap();
        assert!(composed.chars().count() <= MAX_HANDOFF_CHARS);
        assert!(composed.ends_with("[shortened]"));
    }

    #[test]
    fn complete_validates_status_and_summary() {
        let err = apply_complete(
            Some(three_phase_plan()),
            "research",
            PhaseStatus::InProgress,
            None,
            &[],
            now(),
        )
        .unwrap_err();
        assert!(err.contains("status must be"), "{err}");
        let err = apply_complete(
            Some(three_phase_plan()),
            "research",
            PhaseStatus::Completed,
            Some(&"s".repeat(4_001)),
            &[],
            now(),
        )
        .unwrap_err();
        assert!(err.contains("limit is 4000"), "{err}");
        let err = apply_handoff(Some(three_phase_plan()), "research", "  ", now()).unwrap_err();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn handoff_context_follows_depends_on_or_every_completed_earlier_phase() {
        let mut plan = three_phase_plan();
        // Nothing handed off yet.
        assert!(render_handoff_context(&plan, "build").is_none());
        assert!(render_handoff_context(&plan, "verify").is_none());

        // build depends on research: research's hand-off reaches it even while
        // research is still in progress (a pipeline).
        plan = apply_handoff(Some(plan), "research", "RESEARCH RESULT", now()).unwrap();
        let context = render_handoff_context(&plan, "build").unwrap();
        assert!(context.starts_with("## Results from earlier phases"));
        assert!(context.contains("RESEARCH RESULT"));
        // verify names no dependency, so only COMPLETED earlier phases count.
        assert!(render_handoff_context(&plan, "verify").is_none());

        let (completed, _) = apply_complete(
            Some(plan),
            "research",
            PhaseStatus::Completed,
            None,
            &[],
            now(),
        )
        .unwrap();
        let mut plan = completed;
        plan = apply_handoff(Some(plan), "build", "BUILD RESULT", now()).unwrap();
        let context = render_handoff_context(&plan, "verify").unwrap();
        assert!(context.contains("RESEARCH RESULT"));
        assert!(
            !context.contains("BUILD RESULT"),
            "build is not completed yet: {context}"
        );
        let (plan, _) = apply_complete(
            Some(plan),
            "build",
            PhaseStatus::Completed,
            None,
            &[],
            now(),
        )
        .unwrap();
        let context = render_handoff_context(&plan, "verify").unwrap();
        let research_at = context.find("RESEARCH RESULT").unwrap();
        let build_at = context.find("BUILD RESULT").unwrap();
        assert!(research_at < build_at, "plan order: {context}");
        // The first phase has nothing before it.
        assert!(render_handoff_context(&plan, "research").is_none());
        assert!(render_handoff_context(&plan, "unknown").is_none());
    }

    #[test]
    fn handoff_context_is_capped() {
        let phases: Vec<PlanPhaseInput> = (0..5)
            .map(|i| input(&format!("p{i}"), "research", &[]))
            .collect();
        let mut plan = apply_plan(None, Some("big"), None, &phases).unwrap();
        for i in 0..4 {
            let id = format!("p{i}");
            plan = apply_handoff(Some(plan), &id, &"h".repeat(MAX_HANDOFF_CHARS), now()).unwrap();
            plan = apply_complete(Some(plan), &id, PhaseStatus::Completed, None, &[], now())
                .unwrap()
                .0;
        }
        let context = render_handoff_context(&plan, "p4").unwrap();
        assert_eq!(context.chars().count(), HANDOFF_CONTEXT_MAX_CHARS);
        assert!(context.ends_with("[shortened]"));
    }

    #[test]
    fn spawn_checks_name_the_open_phases() {
        let err = check_spawn_phase(None, "research").unwrap_err();
        assert!(err.contains("no orchestration plan"), "{err}");
        assert!(err.contains("Nothing was created"), "{err}");

        let plan = three_phase_plan();
        assert!(check_spawn_phase(Some(&plan), "build").is_ok());
        let err = check_spawn_phase(Some(&plan), "deploy").unwrap_err();
        assert!(err.contains("research, build, verify"), "{err}");

        let (plan, _) = apply_complete(
            Some(plan),
            "research",
            PhaseStatus::Completed,
            None,
            &[],
            now(),
        )
        .unwrap();
        let (plan, _) =
            apply_complete(Some(plan), "verify", PhaseStatus::Skipped, None, &[], now()).unwrap();
        let err = check_spawn_phase(Some(&plan), "research").unwrap_err();
        assert!(err.contains("already completed"), "{err}");
        assert!(err.contains("spawn into: build."), "{err}");
        let err = check_spawn_phase(Some(&plan), "verify").unwrap_err();
        assert!(err.contains("already skipped"), "{err}");
    }

    #[test]
    fn record_spawn_appends_once_starts_the_phase_and_respects_the_cap() {
        let plan = three_phase_plan();
        let plan =
            apply_record_spawn(Some(plan), "build", "c1", Some(&"r".repeat(60)), now()).unwrap();
        let build = plan.phase("build").unwrap();
        assert_eq!(build.status, PhaseStatus::InProgress);
        assert_eq!(build.started_at, Some(now()));
        assert_eq!(build.agents.len(), 1);
        assert_eq!(build.agents[0].role.as_ref().unwrap().chars().count(), 40);

        // Idempotent for the same child.
        let again = apply_record_spawn(Some(plan.clone()), "build", "c1", None, now()).unwrap();
        assert_eq!(again.phase("build").unwrap().agents.len(), 1);

        assert!(apply_record_spawn(Some(plan.clone()), "nope", "c2", None, now()).is_err());
        assert!(apply_record_spawn(None, "build", "c2", None, now()).is_err());

        let ids: Vec<String> = (0..MAX_PLAN_AGENTS).map(|i| format!("k{i}")).collect();
        let mut full = three_phase_plan();
        for id in &ids {
            full = apply_record_spawn(Some(full), "research", id, None, now()).unwrap();
        }
        assert!(apply_record_spawn(Some(full.clone()), "build", "over", None, now()).is_err());
        let err = check_spawn_phase(Some(&full), "build").unwrap_err();
        assert!(err.contains("64 subagents"), "{err}");
    }

    /// A spawn checks its phase before its child exists, and a `complete` may
    /// close the phase before the child is recorded: the record refuses then,
    /// rather than adding a running child to a closed phase.
    #[test]
    fn record_spawn_refuses_a_phase_closed_since_the_check() {
        for status in [PhaseStatus::Completed, PhaseStatus::Skipped] {
            let (plan, _) =
                apply_complete(Some(three_phase_plan()), "build", status, None, &[], now())
                    .unwrap();
            let err = apply_record_spawn(Some(plan), "build", "late", None, now()).unwrap_err();
            assert!(
                err.contains(&format!(
                    "was {} while this subagent was being created",
                    status.as_str()
                )),
                "{err}"
            );
        }
        // A blocked phase is not closed: a spawn resumes it, as before.
        let (plan, _) = apply_complete(
            Some(three_phase_plan()),
            "build",
            PhaseStatus::Blocked,
            None,
            &[],
            now(),
        )
        .unwrap();
        let plan = apply_record_spawn(Some(plan), "build", "next", None, now()).unwrap();
        assert_eq!(plan.phase("build").unwrap().status, PhaseStatus::InProgress);
    }

    /// The guard a spawn holds, as `complete` reads it: in flight while held,
    /// and a mark that moves with every spawn begun, so a spawn that began and
    /// ended between `complete`'s two reads is still seen.
    #[test]
    fn a_phase_spawn_guard_holds_the_barrier_while_it_lives() {
        // Unique: the registry is process-global.
        let parent = format!("guard-parent-{}", uuid::Uuid::new_v4());
        let idle = phase_spawn_mark(&parent, "build");
        assert_eq!(idle, PhaseSpawnMark::default());
        assert!(refuse_racing_spawns("build", idle, idle).is_ok());

        let first = begin_phase_spawn(&parent, " build ");
        let second = begin_phase_spawn(&parent, "build");
        let busy = phase_spawn_mark(&parent, "build");
        assert_eq!(busy.in_flight, 2, "the id is trimmed like every phase id");
        assert_eq!(
            phase_spawn_mark(&parent, "research"),
            PhaseSpawnMark::default(),
            "another phase is not held"
        );
        let err = refuse_racing_spawns("build", idle, busy).unwrap_err();
        assert!(err.contains("has not returned"), "{err}");
        assert!(err.contains("Nothing was changed"), "{err}");
        assert!(refuse_racing_spawns("build", busy, busy).is_err());

        drop(first);
        drop(second);
        let after = phase_spawn_mark(&parent, "build");
        assert_eq!(after.in_flight, 0);
        assert_eq!(after.last_begun, busy.last_begun, "dropping begins nothing");
        // Begun and ended between the two reads: still a race.
        let err = refuse_racing_spawns("build", idle, after).unwrap_err();
        assert!(err.contains("while this call ran"), "{err}");
        assert!(refuse_racing_spawns("build", after, after).is_ok());
    }

    /// `fresh:true` is the way past the limits: it replaces a plan whose every
    /// phase is closed, and is refused while any phase is open.
    #[test]
    fn a_finished_plan_can_be_replaced_with_a_fresh_one() {
        let ids: Vec<String> = (0..MAX_PLAN_AGENTS).map(|i| format!("k{i}")).collect();
        let mut full = three_phase_plan();
        for id in &ids {
            full = apply_record_spawn(Some(full), "research", id, None, now()).unwrap();
        }
        let err = check_spawn_phase(Some(&full), "build").unwrap_err();
        assert!(
            err.contains("fresh:true"),
            "the refusal names the way out: {err}"
        );
        assert!(err.contains("build, verify"), "{err}");

        // Open phases: refused, naming them.
        let err = apply_fresh_plan(
            Some(full.clone()),
            None,
            None,
            &[input("verify-2", "verify", &[])],
        )
        .unwrap_err();
        assert!(err.contains("still open: research, build, verify"), "{err}");

        // A plain revision still cannot drop a phase that ran subagents.
        let done: Vec<PhaseAgentState> = ids
            .iter()
            .map(|id| state(id, SubagentRunStatus::Completed, None))
            .collect();
        let mut finished = full;
        for phase in ["research", "build", "verify"] {
            let checked: &[PhaseAgentState] = if phase == "research" { &done } else { &[] };
            let (next, _) = apply_complete(
                Some(finished),
                phase,
                PhaseStatus::Completed,
                None,
                checked,
                now(),
            )
            .unwrap();
            finished = next;
        }
        let err = apply_plan(
            Some(finished.clone()),
            None,
            None,
            &[input("verify-2", "verify", &[])],
        )
        .unwrap_err();
        assert!(err.contains("start a new plan with fresh:true"), "{err}");

        let fresh = apply_fresh_plan(
            Some(finished),
            None,
            None,
            &[
                input("verify-2", "verify", &[]),
                input("research", "research", &[]),
            ],
        )
        .unwrap();
        assert_eq!(fresh.title, "Ship the parser", "the title is kept");
        assert_eq!(fresh.goal, None, "a new plan's goal is its own");
        assert_eq!(fresh.agent_count(), 0, "the limit counts afresh");
        let research = fresh.phase("research").unwrap();
        assert_eq!(
            research.status,
            PhaseStatus::Pending,
            "a repeated id starts empty"
        );
        assert!(research.handoff.is_none());
        assert!(check_spawn_phase(Some(&fresh), "verify-2").is_ok());
        // No plan yet: fresh is just a new plan.
        assert!(apply_fresh_plan(None, Some("New"), None, &[input("a", "plan", &[])]).is_ok());

        // Twelve phases that ran subagents: the phase limit's way out is the same.
        let twelve: Vec<PlanPhaseInput> = (0..MAX_PHASES)
            .map(|i| input(&format!("p{i}"), "other", &[]))
            .collect();
        let mut wide = apply_plan(None, Some("wide"), None, &twelve).unwrap();
        let mut thirteen = twelve.clone();
        thirteen.push(input("p12", "other", &[]));
        let err = apply_plan(Some(wide.clone()), None, None, &thirteen).unwrap_err();
        assert!(err.contains("fresh:true"), "{err}");
        for i in 0..MAX_PHASES {
            let id = format!("p{i}");
            let child = format!("c{i}");
            wide = apply_record_spawn(Some(wide), &id, &child, None, now()).unwrap();
            let checked = [state(&child, SubagentRunStatus::Completed, None)];
            let (next, _) = apply_complete(
                Some(wide),
                &id,
                PhaseStatus::Completed,
                None,
                &checked,
                now(),
            )
            .unwrap();
            wide = next;
        }
        assert!(apply_fresh_plan(Some(wide), None, None, &[input("p12", "other", &[])]).is_ok());
    }

    #[test]
    fn snapshot_counts_come_from_the_roster_rows() {
        let plan = with_agents(
            three_phase_plan(),
            "research",
            &["a", "b", "c", "d", "e", "f", "g", "stranger"],
        );
        let rows = vec![
            row("a", SubagentRunStatus::Completed, None),
            row("b", SubagentRunStatus::Running, Some(10)),
            row("c", SubagentRunStatus::Queued, Some(400)),
            row("d", SubagentRunStatus::Cancelling, None),
            row("e", SubagentRunStatus::Blocked, Some(999)),
            row("f", SubagentRunStatus::Interrupted, None),
            row("g", SubagentRunStatus::Incomplete, None),
            // A child of this chat that is in no phase is not counted.
            row("loose", SubagentRunStatus::Running, None),
        ];
        let snapshot = snapshot_of(&plan, &rows);
        let research = &snapshot.phases[0];
        assert_eq!(
            research.agent_ids,
            vec!["a", "b", "c", "d", "e", "f", "g"],
            "an agent absent from the roster is dropped"
        );
        assert_eq!(
            research.counts,
            PhaseCounts {
                total: 7,
                done: 1,
                running: 3,
                blocked: 1,
                failed: 2,
                quiet: 1,
            }
        );
        assert_eq!(snapshot.phases[1].counts, PhaseCounts::default());
        assert_eq!(snapshot.title, "Ship the parser");
        assert_eq!(snapshot.revision, plan.revision);
        assert_eq!(
            progress_words(&research.counts),
            "1/7 done, 3 running, 1 quiet, 1 blocked, 2 failed"
        );

        // The hand-off is truncated for display.
        let plan = apply_handoff(Some(plan), "research", &"x".repeat(2_000), now()).unwrap();
        let shown = snapshot_of(&plan, &rows).phases[0].handoff.clone().unwrap();
        assert_eq!(shown.chars().count(), SNAPSHOT_HANDOFF_MAX_CHARS);

        let text = render_snapshot_text(&snapshot);
        assert!(text.contains("1. research [research]"), "{text}");
        let moim = render_moim(&snapshot);
        assert!(
            moim.contains("- research (research): in progress, 1/7 done"),
            "{moim}"
        );
        assert!(
            moim.contains("- build (implementation): pending, 0/0 done"),
            "{moim}"
        );
    }

    #[test]
    fn the_snapshot_serializes_in_the_shape_the_interfaces_read() {
        let snapshot = snapshot_of(
            &with_agents(three_phase_plan(), "build", &["a"]),
            &[row("a", SubagentRunStatus::Running, None)],
        );
        let value = serde_json::to_value(&snapshot).unwrap();
        let build = &value["phases"][1];
        assert_eq!(build["status"], "in_progress");
        assert_eq!(build["kind"], "implementation");
        assert_eq!(build["agentIds"], serde_json::json!(["a"]));
        assert_eq!(build["counts"]["running"], 1);
        assert_eq!(build["dependsOn"], serde_json::json!(["research"]));
        assert!(build.get("startedAt").is_some());
    }

    async fn store_with_parent() -> (tempfile::TempDir, Arc<SessionManager>, String) {
        let temp = tempfile::tempdir().unwrap();
        let manager = Arc::new(SessionManager::new(temp.path().join("sessions")));
        let parent = manager
            .create_session(
                temp.path().to_path_buf(),
                "parent".into(),
                SessionType::User,
            )
            .await
            .unwrap();
        (temp, manager, parent.id)
    }

    #[tokio::test]
    async fn the_plan_round_trips_through_its_own_key_and_leaves_others_alone() {
        let (_temp, manager, parent) = store_with_parent().await;
        assert!(load_plan(&manager, &parent).await.unwrap().is_none());
        assert!(snapshot(&manager, &parent, &[]).await.is_none());
        assert!(validate_spawn_phase(&manager, &parent, "research")
            .await
            .unwrap_err()
            .contains("no orchestration plan"));

        manager
            .update_extension_state(&parent, "todo", "v1", |_| {
                Ok(serde_json::json!({ "items": [] }))
            })
            .await
            .unwrap();

        mutate_plan(&manager, &parent, |current| {
            assert!(current.is_none());
            Ok((three_phase_plan(), ()))
        })
        .await
        .unwrap();
        let stored = load_plan(&manager, &parent).await.unwrap().unwrap();
        assert_eq!(stored.revision, 1, "the store bumps the revision");

        record_spawn(&manager, &parent, "research", "child-1", Some("reader"))
            .await
            .unwrap();
        let stored = load_plan(&manager, &parent).await.unwrap().unwrap();
        assert_eq!(stored.revision, 2);
        assert_eq!(
            stored.phase("research").unwrap().agents[0].child_session_id,
            "child-1"
        );
        assert!(validate_spawn_phase(&manager, &parent, "research")
            .await
            .is_ok());
        assert!(handoff_context_for(&manager, &parent, "build")
            .await
            .is_none());

        // A refused mutation writes nothing.
        let refused = mutate_plan(&manager, &parent, |current| {
            apply_start(current, "nope", now())
        })
        .await;
        assert!(refused
            .unwrap_err()
            .to_string()
            .contains("not in this chat's plan"));
        assert_eq!(
            load_plan(&manager, &parent)
                .await
                .unwrap()
                .unwrap()
                .revision,
            2
        );

        let session = manager.get_session(&parent, false).await.unwrap();
        assert!(
            session
                .extension_data
                .get_extension_state("todo", "v1")
                .is_some(),
            "the plan write must not erase another key"
        );

        // An unknown chat is an error, not a silent no-op.
        assert!(
            record_spawn(&manager, "no-such-chat", "research", "c", None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_plan_this_build_cannot_read_is_preserved_not_replaced() {
        let (_temp, manager, parent) = store_with_parent().await;
        let future = serde_json::json!({ "title": 7, "phases": "from a newer build" });
        manager
            .update_extension_state(&parent, PLAN_STATE, PLAN_VERSION, |_| Ok(future.clone()))
            .await
            .unwrap();
        let err = mutate_plan(&manager, &parent, |_| Ok((three_phase_plan(), ())))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cannot read"), "{err}");
        let stored = manager
            .get_extension_state(&parent, PLAN_STATE, PLAN_VERSION)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored, future);
        assert!(snapshot(&manager, &parent, &[]).await.is_none());
    }

    /// A child of `parent` the way a spawn makes one: a `SubAgent` row that
    /// names its parent. Test-build ids carry a per-store prefix, so they are
    /// unique in the process-global handle registry too.
    async fn spawn_child(manager: &SessionManager, parent: &str, name: &str) -> String {
        let child = manager
            .create_session(
                std::env::temp_dir(),
                format!("Subagent: {name}"),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        manager
            .update(&child.id)
            .parent_session_id(Some(parent.to_string()))
            .apply()
            .await
            .unwrap();
        child.id
    }

    async fn record_outcome_line(manager: &SessionManager, child: &str, line: &str) {
        let line = line.to_string();
        manager
            .update_extension_state(
                child,
                crate::agents::subagent_roster::SUBAGENT_OUTCOME_STATE,
                crate::agents::subagent_roster::SUBAGENT_OUTCOME_VERSION,
                move |_| {
                    Ok(serde_json::json!({
                        "status": "completed",
                        "ended_at": "2026-11-04T12:00:00Z",
                        "result_line": line,
                    }))
                },
            )
            .await
            .unwrap();
    }

    async fn make_private(manager: &SessionManager, id: &str) {
        manager
            .update(id)
            .raise_privacy(SessionClassification::Private, "test:private")
            .apply()
            .await
            .unwrap();
    }

    fn plan_agents(ids: &[&str]) -> Vec<PlanAgent> {
        ids.iter()
            .map(|id| PlanAgent {
                child_session_id: id.to_string(),
                role: None,
                spawned_at: now(),
            })
            .collect()
    }

    #[tokio::test]
    async fn phase_agent_states_reads_live_handles_then_outcome_records() {
        let (_temp, manager, parent) = store_with_parent().await;
        // Real rows (not named in the roster, since they name no parent), so
        // each child's tier can be read and its result line shown.
        let running = manager
            .create_session(std::env::temp_dir(), "live".into(), SessionType::SubAgent)
            .await
            .unwrap()
            .id;
        let finished = manager
            .create_session(std::env::temp_dir(), "done".into(), SessionType::SubAgent)
            .await
            .unwrap()
            .id;
        let live = crate::agents::subagent_handle::BackgroundSubagent::register(
            parent.clone(),
            running.clone(),
            "live",
            tokio_util::sync::CancellationToken::new(),
        );
        let done = crate::agents::subagent_handle::BackgroundSubagent::register(
            parent.clone(),
            finished.clone(),
            "done",
            tokio_util::sync::CancellationToken::new(),
        );
        let mut result = crate::agents::subagent_result::SubagentResult::from_error("boom");
        result.status = crate::agents::subagent_result::SubagentStatus::Completed;
        result.summary = "\nAll tests pass\nsecond line".into();
        done.complete(result);

        let recorded = manager
            .create_session(std::env::temp_dir(), "child".into(), SessionType::SubAgent)
            .await
            .unwrap();
        manager
            .update_extension_state(
                &recorded.id,
                crate::agents::subagent_roster::SUBAGENT_OUTCOME_STATE,
                crate::agents::subagent_roster::SUBAGENT_OUTCOME_VERSION,
                |_| {
                    Ok(serde_json::json!({
                        "status": "failed",
                        "ended_at": "2026-11-04T12:00:00Z",
                        "result_line": "the API refused the key",
                    }))
                },
            )
            .await
            .unwrap();

        let agents: Vec<PlanAgent> = [
            running.as_str(),
            finished.as_str(),
            recorded.id.as_str(),
            "ghost",
        ]
        .iter()
        .map(|id| PlanAgent {
            child_session_id: id.to_string(),
            role: None,
            spawned_at: now(),
        })
        .collect();
        let states = phase_agent_states(&manager, &parent, &agents).await;
        let by_id = |id: &str| states.iter().find(|s| s.child_session_id == id).unwrap();
        assert_eq!(by_id(&running).status, SubagentRunStatus::Running);
        assert_eq!(by_id(&finished).status, SubagentRunStatus::Completed);
        assert_eq!(
            by_id(&finished).result_line.as_deref(),
            Some("All tests pass")
        );
        assert_eq!(by_id(&recorded.id).status, SubagentRunStatus::Failed);
        assert_eq!(
            by_id(&recorded.id).result_line.as_deref(),
            Some("the API refused the key")
        );
        assert_eq!(by_id("ghost").status, SubagentRunStatus::Unknown);
        assert!(
            states.iter().all(|state| !state.withheld),
            "every child is as public as its parent, and the ghost has nothing to hide"
        );

        // Settle the live handle so no other test sees it running.
        live.complete(crate::agents::subagent_result::SubagentResult::from_error(
            "done",
        ));
    }

    /// A child ratcheted private mid-run while its parent stayed public: its
    /// result line and its session name never reach the parent's hand-off,
    /// and the hand-off says how many results it left out. Once the parent is
    /// private too, nothing is withheld.
    #[tokio::test]
    async fn a_child_more_private_than_its_parent_is_withheld_from_the_handoff() {
        let (_temp, manager, parent) = store_with_parent().await;
        let open = spawn_child(&manager, &parent, "open reader").await;
        let secret = spawn_child(&manager, &parent, "patient MRN 4471").await;
        record_outcome_line(&manager, &open, "Read all three modules").await;
        record_outcome_line(&manager, &secret, "Cohort has 12 MS patients").await;
        make_private(&manager, &secret).await;

        let states = phase_agent_states(
            &manager,
            &parent,
            &plan_agents(&[open.as_str(), secret.as_str()]),
        )
        .await;
        let by_id = |id: &str| states.iter().find(|s| s.child_session_id == id).unwrap();
        assert!(!by_id(&open).withheld);
        assert_eq!(
            by_id(&open).result_line.as_deref(),
            Some("Read all three modules")
        );
        assert_eq!(
            by_id(&open).label,
            "open reader",
            "a visible child keeps its name"
        );
        assert!(by_id(&secret).withheld);
        assert_eq!(by_id(&secret).result_line, None);
        assert_eq!(by_id(&secret).status, SubagentRunStatus::Completed);
        assert_eq!(
            by_id(&secret).label,
            secret,
            "a withheld child is named by its id, never its session name"
        );

        let composed = compose_auto_handoff(&states).unwrap();
        assert!(composed.contains("Read all three modules"), "{composed}");
        assert!(!composed.contains("12 MS patients"), "{composed}");
        assert!(!composed.contains("MRN"), "{composed}");
        assert!(composed.contains("1 result was withheld"), "{composed}");
        assert!(
            composed.contains(&format!("({secret}): (result withheld)")),
            "{composed}"
        );

        // The barrier's reply says it too, and the stored hand-off is what the
        // snapshot (MOIM, panel, tool reply) and later phases read.
        let plan = with_agents(
            three_phase_plan(),
            "research",
            &[open.as_str(), secret.as_str()],
        );
        let (closed, note) = apply_complete(
            Some(plan),
            "research",
            PhaseStatus::Completed,
            None,
            &states,
            now(),
        )
        .unwrap();
        assert!(note.contains("1 result was withheld"), "{note}");
        let stored = closed.phase("research").unwrap().handoff.clone().unwrap();
        assert!(!stored.summary.contains("12 MS patients"));
        let rows: Vec<SubagentRow> = Vec::new();
        let snapshot = snapshot_of(&closed, &rows);
        let shown = render_snapshot_text(&snapshot);
        assert!(!shown.contains("12 MS patients"), "{shown}");
        let context = render_handoff_context(&closed, "build").unwrap();
        assert!(!context.contains("12 MS patients"), "{context}");

        make_private(&manager, &parent).await;
        let states = phase_agent_states(
            &manager,
            &parent,
            &plan_agents(&[open.as_str(), secret.as_str()]),
        )
        .await;
        assert!(states.iter().all(|state| !state.withheld));
        let composed = compose_auto_handoff(&states).unwrap();
        assert!(composed.contains("12 MS patients"), "{composed}");
        assert!(!composed.contains("withheld"), "{composed}");
    }

    /// A child whose tier cannot be read is withheld when it has a result to
    /// show (fail closed), and two withheld results are counted as two.
    #[test]
    fn withheld_results_are_counted_and_never_shown() {
        let mut first = state("c1", SubagentRunStatus::Completed, None);
        first.withheld = true;
        let mut second = state("c2", SubagentRunStatus::Failed, None);
        second.withheld = true;
        let visible = state("c3", SubagentRunStatus::Completed, Some("fine"));
        let composed = compose_auto_handoff(&[first, second, visible]).unwrap();
        assert!(composed.contains("2 results were withheld"), "{composed}");
        assert!(composed.contains("c1 role [completed] (c1): (result withheld)"));
        assert!(composed.contains("c2 role [failed] (c2): (result withheld)"));
        assert!(composed.contains("c3 role [completed] (c3): fine"));
        let heading = composed.lines().next().unwrap();
        assert!(
            heading.contains("withheld"),
            "the count sits in the heading, where the length cap cannot cut it: {heading}"
        );
    }

    #[tokio::test]
    async fn an_unreadable_child_tier_withholds_only_what_it_would_show() {
        let (_temp, manager, parent) = store_with_parent().await;
        // Unique ids: the handle registry is process-global.
        let rowless = format!("orch-rowless-{}", uuid::Uuid::new_v4());
        let done = crate::agents::subagent_handle::BackgroundSubagent::register(
            parent.clone(),
            rowless.clone(),
            "rowless",
            tokio_util::sync::CancellationToken::new(),
        );
        let mut result = crate::agents::subagent_result::SubagentResult::from_error("boom");
        result.status = crate::agents::subagent_result::SubagentStatus::Completed;
        result.summary = "Something the parent may not see".into();
        done.complete(result);

        let states = phase_agent_states(
            &manager,
            &parent,
            &plan_agents(&[rowless.as_str(), "ghost"]),
        )
        .await;
        let by_id = |id: &str| states.iter().find(|s| s.child_session_id == id).unwrap();
        assert!(by_id(&rowless).withheld, "no row to read a tier from");
        assert_eq!(by_id(&rowless).result_line, None);
        assert!(
            !by_id("ghost").withheld,
            "nothing to show, so nothing is withheld"
        );
    }

    // -----------------------------------------------------------------------
    // Decision gates
    // -----------------------------------------------------------------------

    fn gate_input(questions: &[(&str, &str)]) -> PhaseGateInput {
        PhaseGateInput {
            questions: questions
                .iter()
                .map(|(name, text)| (name.to_string(), text.to_string()))
                .collect(),
            threshold: None,
            max_attempts: None,
            allow_uncalibrated: None,
            model: None,
        }
    }

    fn gated(
        id: &str,
        kind: &str,
        depends_on: &[&str],
        gate: Option<PhaseGateInput>,
    ) -> OrchestratePhaseInput {
        OrchestratePhaseInput {
            phase: input(id, kind, depends_on),
            gate,
            monitors: None,
        }
    }

    /// The gate [`gated_plan`] puts on "build".
    fn build_gate_input() -> PhaseGateInput {
        gate_input(&[
            ("tests_pass", "Do the results show every new test passing?"),
            ("scope_met", "Did the phase deliver every item in its goal?"),
        ])
    }

    /// research, then build (gated, depending on research), then verify,
    /// which names no dependency.
    fn held_plan() -> OrchestrationPlan {
        apply_gated_plan(None, Some("Ship the parser"), None, &held_revision()).unwrap()
    }

    /// [`held_plan`]'s phases as a revision lists them, with the gate.
    fn held_revision() -> Vec<OrchestratePhaseInput> {
        vec![
            gated("research", "research", &[], None),
            gated(
                "build",
                "implement",
                &["research"],
                Some(build_gate_input()),
            ),
            gated("verify", "verify", &[], None),
        ]
    }

    fn person_spec(questions: &[(&str, &str)], threshold: Option<f64>) -> PhaseGateSpec {
        PhaseGateSpec {
            questions: questions
                .iter()
                .map(|(name, text)| (name.to_string(), text.to_string()))
                .collect(),
            threshold,
            max_attempts: None,
            model: None,
            allow_uncalibrated: None,
        }
    }

    fn person_gate(phase: &str) -> PhaseGate {
        validate_person_gate(
            phase,
            &person_spec(&[("cited", "Is every claim cited?")], None),
        )
        .unwrap()
    }

    /// The gate on "build" passes, closing the phase.
    fn pass_build(plan: OrchestrationPlan) -> OrchestrationPlan {
        let gate = stored_gate(&plan);
        let pass = GateVerdict::Passed(answers(&[("tests_pass", 0.9), ("scope_met", 0.9)], 0.8));
        apply_gate_verdict(Some(plan), "build", &gate, &pass, None, &[], now())
            .unwrap()
            .0
    }

    fn monitor_spec(name: &str) -> MonitorSpec {
        serde_json::from_value(serde_json::json!({
            "name": name,
            "question": "Is the agent editing files outside src/?",
            "threshold": 0.8,
            "action": "stop",
        }))
        .unwrap()
    }

    /// A plan whose "build" phase carries a two-question gate.
    fn gated_plan() -> OrchestrationPlan {
        apply_gated_plan(
            None,
            Some("Ship the parser"),
            None,
            &[
                gated("research", "research", &[], None),
                gated(
                    "build",
                    "implement",
                    &["research"],
                    Some(gate_input(&[
                        ("tests_pass", "Do the results show every new test passing?"),
                        ("scope_met", "Did the phase deliver every item in its goal?"),
                    ])),
                ),
            ],
        )
        .unwrap()
    }

    fn stored_gate(plan: &OrchestrationPlan) -> PhaseGate {
        plan.phase("build").unwrap().gate.clone().unwrap()
    }

    fn answers(ps: &[(&str, f64)], threshold: f64) -> Vec<GateAnswer> {
        ps.iter()
            .map(|(name, p)| GateAnswer {
                question: name.to_string(),
                instructions: format!("{name}?"),
                p_yes: *p,
                passed: *p >= threshold,
            })
            .collect()
    }

    #[test]
    fn a_gate_is_validated_and_takes_its_defaults() {
        let plan = gated_plan();
        let gate = stored_gate(&plan);
        assert_eq!(gate.threshold, DEFAULT_GATE_THRESHOLD);
        assert_eq!(gate.max_attempts, 3);
        assert!(!gate.allow_uncalibrated);
        assert_eq!(gate.questions.len(), 2);
        assert!(gate
            .questions
            .values()
            .all(|q| q.kind == DecisionKind::Binary {} && q.threshold.is_none()));

        let nine: Vec<(String, String)> =
            (0..9).map(|i| (format!("q{i}"), "yes?".into())).collect();
        let nine: Vec<(&str, &str)> = nine.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let cases: Vec<(PhaseGateInput, &str)> = vec![
            (gate_input(&[]), "1 to 8"),
            (gate_input(&nine), "1 to 8"),
            (gate_input(&[("bad name", "yes?")]), "bad name"),
            (gate_input(&[("q", "  ")]), "instructions"),
            (
                PhaseGateInput {
                    threshold: Some(1.5),
                    ..gate_input(&[("q", "yes?")])
                },
                "from 0 to 1",
            ),
            (
                PhaseGateInput {
                    max_attempts: Some(0),
                    ..gate_input(&[("q", "yes?")])
                },
                "1 to 5 attempts",
            ),
            (
                PhaseGateInput {
                    max_attempts: Some(6),
                    ..gate_input(&[("q", "yes?")])
                },
                "1 to 5 attempts",
            ),
            (
                PhaseGateInput {
                    model: Some("tev1".into()),
                    ..gate_input(&[("q", "yes?")])
                },
                "provider/model",
            ),
        ];
        for (gate, expected) in cases {
            let err = validate_gate("build", &gate).unwrap_err();
            assert!(err.contains(expected), "expected {expected:?} in {err:?}");
            assert!(err.contains("\"build\""), "{err}");
        }
        let named = validate_gate(
            "build",
            &PhaseGateInput {
                model: Some("ollama_decision/tev1:0.8b".into()),
                threshold: Some(0.9),
                max_attempts: Some(5),
                allow_uncalibrated: Some(true),
                ..gate_input(&[("q", "yes?")])
            },
        )
        .unwrap();
        assert_eq!(
            named.model,
            Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b"))
        );
        assert!(named.allow_uncalibrated);
    }

    /// A stored gate binds: a revision may leave it out or repeat it
    /// unchanged (both keep its owner and record), and any other gate for the
    /// phase is refused by name, whoever set the stored one.
    #[test]
    fn a_revision_cannot_change_or_reset_a_gate() {
        let mut plan = gated_plan();
        let gate = stored_gate(&plan);
        plan.phase_mut("build").unwrap().gate_state = Some(GateState {
            attempts: 2,
            last_passed: Some(false),
            ..GateState::default()
        });
        let revise = |plan: &OrchestrationPlan, gate: Option<PhaseGateInput>| {
            apply_gated_plan(
                Some(plan.clone()),
                None,
                None,
                &[
                    gated("research", "research", &[], None),
                    gated("build", "implement", &["research"], gate),
                ],
            )
        };

        let kept = revise(&plan, None).unwrap();
        let build = kept.phase("build").unwrap();
        assert_eq!(build.gate.as_ref(), Some(&gate));
        assert_eq!(build.gate_state.as_ref().unwrap().attempts, 2);

        let same = revise(&plan, Some(build_gate_input())).unwrap();
        let build = same.phase("build").unwrap();
        assert_eq!(build.gate.as_ref(), Some(&gate));
        assert_eq!(
            build.gate_state.as_ref().unwrap().attempts,
            2,
            "the same gate declared again keeps its record"
        );
        let reordered = gate_input(&[
            ("scope_met", "Did the phase deliver every item in its goal?"),
            ("tests_pass", "Do the results show every new test passing?"),
        ]);
        assert!(
            revise(&plan, Some(reordered)).is_ok(),
            "the same questions in another order are the same gate"
        );

        let changes = [
            PhaseGateInput {
                threshold: Some(0.0),
                ..build_gate_input()
            },
            PhaseGateInput {
                threshold: Some(0.6),
                ..build_gate_input()
            },
            gate_input(&[("tests_pass", "Do the tests pass?")]),
            PhaseGateInput {
                max_attempts: Some(5),
                ..build_gate_input()
            },
            PhaseGateInput {
                model: Some("ollama_decision/tev1:0.8b".into()),
                ..build_gate_input()
            },
            PhaseGateInput {
                allow_uncalibrated: Some(true),
                ..build_gate_input()
            },
        ];
        for changed in changes {
            let err = revise(&plan, Some(changed.clone())).unwrap_err();
            assert!(
                err.starts_with("Phase \"build\" already has a gate, and a revision cannot"),
                "{changed:?}: {err}"
            );
            assert!(err.contains("`biorouter session gates`"), "{err}");
            assert!(err.ends_with("Nothing was changed."), "{err}");
        }

        // The refusal says who set a gate that is not the agent's own.
        for (owner, phrase) in [
            (
                GateOwner::Person,
                "already has a gate, set by the person, and",
            ),
            (
                GateOwner::Workflow,
                "already has a gate, set by the workflow, and",
            ),
        ] {
            let mut owned = plan.clone();
            owned
                .phase_mut("build")
                .unwrap()
                .gate
                .as_mut()
                .unwrap()
                .owner = owner;
            let err = revise(&owned, Some(gate_input(&[("q", "Done?")]))).unwrap_err();
            assert!(err.contains(phrase), "{err}");
            let kept = revise(&owned, Some(build_gate_input())).unwrap();
            assert_eq!(
                kept.phase("build").unwrap().gate.as_ref().unwrap().owner,
                owner,
                "repeating a gate keeps its owner"
            );
        }

        // A completed phase's gate cannot be rewritten either.
        let closed = pass_build(plan);
        assert!(revise(&closed, Some(gate_input(&[("q", "Done?")]))).is_err());
    }

    #[test]
    fn a_gate_the_agent_declares_needs_at_least_one_half() {
        let with_threshold = |threshold: f64| PhaseGateInput {
            threshold: Some(threshold),
            ..gate_input(&[("done", "Is the phase done?")])
        };
        let declare = |threshold: f64| {
            apply_gated_plan(
                None,
                Some("t"),
                None,
                &[gated(
                    "build",
                    "implement",
                    &[],
                    Some(with_threshold(threshold)),
                )],
            )
        };
        let err = declare(0.49).unwrap_err();
        assert_eq!(
            err,
            "Phase \"build\"'s gate threshold is 0.49; a gate you declare needs at least 0.50, \
             because below that it passes a phase the decision model thinks more likely \
             unfinished than done."
        );
        assert!(declare(0.5).is_ok());
        assert!(declare(1.0).is_ok());

        // A gate added to an ungated phase in a revision gets the floor too.
        let err = apply_gated_plan(
            Some(three_phase_plan()),
            None,
            None,
            &[
                gated("research", "research", &[], None),
                gated(
                    "build",
                    "implement",
                    &["research"],
                    Some(with_threshold(0.3)),
                ),
            ],
        )
        .unwrap_err();
        assert!(err.contains("threshold is 0.30"), "{err}");

        // A workflow's gate has no floor, and the agent may repeat it as is.
        let workflow = apply_workflow_plan(
            "t",
            None,
            vec![gated("build", "implement", &[], Some(with_threshold(0.2)))],
            now(),
        )
        .unwrap();
        let gate = workflow.phase("build").unwrap().gate.clone().unwrap();
        assert_eq!((gate.threshold, gate.owner), (0.2, GateOwner::Workflow));
        let repeated = apply_gated_plan(
            Some(workflow),
            None,
            None,
            &[gated("build", "implement", &[], Some(with_threshold(0.2)))],
        )
        .unwrap();
        assert_eq!(repeated.phase("build").unwrap().gate.as_ref(), Some(&gate));

        // The person's gate has no floor either, and is the person's.
        let person =
            validate_person_gate("build", &person_spec(&[("q", "Done?")], Some(0.2))).unwrap();
        assert_eq!((person.threshold, person.owner), (0.2, GateOwner::Person));
        assert_eq!(
            validate_person_gate("build", &person_spec(&[("q", "Done?")], Some(0.0)))
                .unwrap()
                .threshold,
            0.0
        );
        // It shares every other check.
        for (spec, expected) in [
            (person_spec(&[], None), "1 to 8"),
            (person_spec(&[("q", "Done?")], Some(1.5)), "from 0 to 1"),
            (person_spec(&[("bad name", "Done?")], None), "bad name"),
            (
                PhaseGateSpec {
                    max_attempts: Some(6),
                    ..person_spec(&[("q", "Done?")], None)
                },
                "1 to 5 attempts",
            ),
            (
                PhaseGateSpec {
                    model: Some(DecisionModelRef::new("", "tev1")),
                    ..person_spec(&[("q", "Done?")], None)
                },
                "both a provider and a model",
            ),
        ] {
            let err = validate_person_gate("build", &spec).unwrap_err();
            assert!(err.contains(expected), "expected {expected:?} in {err:?}");
        }
        // The route's body refuses a key it does not know.
        let typo: Result<PhaseGateSpec, _> = serde_json::from_value(serde_json::json!({
            "questions": {"q": "Done?"},
            "treshold": 0.5,
        }));
        assert!(typo.is_err());
    }

    #[test]
    fn a_revision_cannot_drop_a_gated_phase_even_before_it_ran() {
        let plan = held_plan();
        assert!(plan.phase("build").unwrap().agents.is_empty());
        let err = apply_gated_plan(
            Some(plan.clone()),
            None,
            None,
            &[
                gated("research", "research", &[], None),
                gated("verify", "verify", &[], None),
            ],
        )
        .unwrap_err();
        assert_eq!(
            err,
            "This revision removes build, which has a gate. A gated phase stays in the plan \
             until its gate passes or the person decides, so keep it in the list. Nothing was \
             changed."
        );

        // Two gated phases are named together.
        let mut two = plan.clone();
        two.phase_mut("verify").unwrap().gate = Some(stored_gate(&plan));
        let err = apply_gated_plan(
            Some(two),
            None,
            None,
            &[gated("research", "research", &[], None)],
        )
        .unwrap_err();
        assert!(
            err.contains("removes build, verify, which have a gate"),
            "{err}"
        );

        // Once its gate passed, the phase holds nothing and may go.
        let revised = apply_gated_plan(
            Some(pass_build(plan)),
            None,
            None,
            &[
                gated("research", "research", &[], None),
                gated("verify", "verify", &[], None),
            ],
        )
        .unwrap();
        assert!(revised.phase("build").is_none());
    }

    #[test]
    fn a_gated_open_phase_keeps_its_title_goal_and_kind() {
        let mut plan = held_plan();
        plan.phase_mut("build").unwrap().goal = Some("A parser with tests".into());
        let revise = |plan: &OrchestrationPlan, build: PlanPhaseInput| {
            apply_gated_plan(
                Some(plan.clone()),
                None,
                None,
                &[
                    gated("research", "research", &[], None),
                    OrchestratePhaseInput {
                        phase: build,
                        gate: None,
                        monitors: None,
                    },
                    gated("verify", "verify", &[], None),
                ],
            )
        };
        let base = || PlanPhaseInput {
            goal: Some("A parser with tests".into()),
            ..input("build", "implement", &["research"])
        };
        let changes = [
            PlanPhaseInput {
                title: "Something easier".into(),
                ..base()
            },
            PlanPhaseInput {
                goal: Some("A parser".into()),
                ..base()
            },
            PlanPhaseInput {
                kind: Some("review".into()),
                ..base()
            },
        ];
        for changed in changes {
            let err = revise(&plan, changed).unwrap_err();
            assert_eq!(
                err,
                "Phase \"build\" has a gate, so its title, goal and kind stay as they were \
                 while it is open: its gate reads them. Nothing was changed."
            );
        }

        // Its dependencies stay editable, and a goal or kind left out is kept.
        let revised = revise(
            &plan,
            PlanPhaseInput {
                kind: None,
                goal: None,
                ..input("build", "implement", &[])
            },
        )
        .unwrap();
        let build = revised.phase("build").unwrap();
        assert_eq!(build.goal.as_deref(), Some("A parser with tests"));
        assert_eq!(build.kind, PhaseKind::Implementation);
        assert!(build.depends_on.is_empty());
        assert!(
            revise(&plan, base()).is_ok(),
            "the same words are no change"
        );

        // Ungated phases, and a gated phase once closed, are free to change.
        let renamed = PlanPhaseInput {
            title: "Read everything".into(),
            ..input("research", "research", &[])
        };
        let ok = apply_gated_plan(
            Some(plan.clone()),
            None,
            None,
            &[
                OrchestratePhaseInput {
                    phase: renamed,
                    gate: None,
                    monitors: None,
                },
                gated("build", "implement", &["research"], None),
                gated("verify", "verify", &[], None),
            ],
        );
        assert!(ok.is_ok(), "{ok:?}");
        let closed = pass_build(plan);
        assert!(revise(
            &closed,
            PlanPhaseInput {
                title: "Built".into(),
                ..base()
            }
        )
        .is_ok());
    }

    /// C-5: a held phase is a refusal about the plan's state, not about the
    /// call, so it must not be reported as rejected arguments. A phase the plan
    /// does not name, or a spawn with no plan, still is the call's fault.
    #[test]
    fn a_held_spawn_is_not_a_fault_in_the_call() {
        let plan = held_plan();
        let held = classify_spawn_phase(Some(&plan), "verify").unwrap_err();
        assert!(!held.fix_the_call, "{held:?}");
        assert!(held.sentence.contains("waits for phase \"build\"'s gate"));
        let unknown = classify_spawn_phase(Some(&plan), "nope").unwrap_err();
        assert!(unknown.fix_the_call, "{unknown:?}");
        let no_plan = classify_spawn_phase(None, "verify").unwrap_err();
        assert!(no_plan.fix_the_call, "{no_plan:?}");
        assert!(classify_spawn_phase(Some(&plan), "build").is_ok());
    }

    /// An open gate holds every phase listed after it, by position: "verify"
    /// names no dependency and still waits.
    #[test]
    fn phases_after_an_open_gate_are_held() {
        let plan = held_plan();
        assert_eq!(hold_of(&plan, "verify"), Some("build"));
        assert_eq!(hold_of(&plan, "build"), None, "a gate does not hold itself");
        assert_eq!(hold_of(&plan, "research"), None);
        assert_eq!(hold_of(&plan, "nope"), None);
        let held = "Phase \"verify\" waits for phase \"build\"'s gate: it cannot start, take a \
                    subagent or complete until that gate passes or the person passes or skips \
                    \"build\". Work on \"build\" first. Phases you can work on now: research, \
                    build.";

        assert_eq!(
            apply_start(Some(plan.clone()), "verify", now()).unwrap_err(),
            held
        );
        assert_eq!(
            check_spawn_phase(Some(&plan), "verify").unwrap_err(),
            format!("{held} Nothing was created.")
        );
        for complete in [apply_complete, apply_agent_complete] {
            let err = complete(
                Some(plan.clone()),
                "verify",
                PhaseStatus::Completed,
                None,
                &[],
                now(),
            )
            .unwrap_err();
            assert_eq!(err, held);
        }
        // A held phase with no gate of its own may still be skipped or blocked.
        for status in [PhaseStatus::Skipped, PhaseStatus::Blocked] {
            let (next, _) =
                apply_agent_complete(Some(plan.clone()), "verify", status, None, &[], now())
                    .unwrap();
            assert_eq!(next.phase("verify").unwrap().status, status);
        }
        // The gated phase itself is free to work on, and a refusal names only
        // the phases the agent can work on.
        assert!(apply_start(Some(plan.clone()), "build", now()).is_ok());
        assert!(check_spawn_phase(Some(&plan), "build").is_ok());
        let err = check_spawn_phase(Some(&plan), "deploy").unwrap_err();
        assert!(err.contains("spawn into: research, build."), "{err}");

        let snapshot = snapshot_of(&plan, &[]);
        assert_eq!(snapshot.phases[2].held_by.as_deref(), Some("build"));
        assert_eq!(snapshot.phases[1].held_by, None);
        assert_eq!(
            snapshot.phases[1].gate.as_ref().unwrap().holds,
            vec!["verify"]
        );

        // A gate on an earlier phase holds a later gated phase too; each held
        // phase names the first gate before it.
        let mut two = plan.clone();
        two.phase_mut("verify").unwrap().gate = Some(stored_gate(&plan));
        assert_eq!(
            gate_decision_needed(&two, "verify"),
            GatePath::Held(held.to_string())
        );
        assert_eq!(gate_decision_needed(&two, "build"), GatePath::Ask);
        // Every door gives a held gated phase the hold as its reason, not
        // its own unasked gate.
        for complete in [apply_complete, apply_agent_complete] {
            let err = complete(
                Some(two.clone()),
                "verify",
                PhaseStatus::Completed,
                None,
                &[],
                now(),
            )
            .unwrap_err();
            assert_eq!(err, held);
        }

        // Once its gate passes, nothing waits.
        let passed = pass_build(plan);
        assert_eq!(hold_of(&passed, "verify"), None);
        assert!(apply_start(Some(passed.clone()), "verify", now()).is_ok());
        assert!(check_spawn_phase(Some(&passed), "verify").is_ok());
        assert_eq!(snapshot_of(&passed, &[]).phases[2].held_by, None);
    }

    #[test]
    fn a_revision_cannot_move_a_held_phase_ahead_of_its_gate() {
        let plan = held_plan();
        let err = apply_gated_plan(
            Some(plan.clone()),
            None,
            None,
            &[
                gated("research", "research", &[], None),
                gated("verify", "verify", &[], None),
                gated("build", "implement", &["research"], None),
            ],
        )
        .unwrap_err();
        assert_eq!(
            err,
            "Phase \"verify\" waits for phase \"build\"'s gate and cannot be moved ahead of it. \
             Nothing was changed."
        );

        // A new phase may go anywhere, and an earlier phase may move after it.
        let added = apply_gated_plan(
            Some(plan.clone()),
            None,
            None,
            &[
                gated("lint", "testing", &[], None),
                gated("build", "implement", &[], None),
                gated("research", "research", &[], None),
                gated("verify", "verify", &[], None),
            ],
        )
        .unwrap();
        assert_eq!(hold_of(&added, "research"), Some("build"));
        assert_eq!(hold_of(&added, "lint"), None);

        // A closed phase waits for nothing, so it may move ahead.
        let (skipped, _) = apply_agent_complete(
            Some(plan.clone()),
            "verify",
            PhaseStatus::Skipped,
            None,
            &[],
            now(),
        )
        .unwrap();
        assert!(apply_gated_plan(
            Some(skipped),
            None,
            None,
            &[
                gated("verify", "verify", &[], None),
                gated("research", "research", &[], None),
                gated("build", "implement", &["research"], None),
            ],
        )
        .is_ok());

        // Once the gate passed, the order is free again.
        let passed = pass_build(plan);
        assert!(apply_gated_plan(
            Some(passed),
            None,
            None,
            &[
                gated("verify", "verify", &[], None),
                gated("research", "research", &[], None),
                gated("build", "implement", &["research"], None),
            ],
        )
        .is_ok());
    }

    /// A spawn checks its phase before its child exists; a gate the person
    /// sets on an earlier phase in between refuses the record, in the same
    /// transaction that would have written it.
    #[tokio::test]
    async fn record_spawn_rechecks_the_hold_in_its_transaction() {
        let plan = three_phase_plan();
        assert!(check_spawn_phase(Some(&plan), "verify").is_ok());
        let (pinned, _) =
            apply_person_set_gate(Some(plan.clone()), "build", person_gate("build"), now())
                .unwrap();
        let err =
            apply_record_spawn(Some(pinned.clone()), "verify", "late", None, now()).unwrap_err();
        assert!(
            err.starts_with("Phase \"verify\" waits for phase \"build\"'s gate"),
            "{err}"
        );
        // A child recorded before the gate stays recorded.
        let recorded = apply_record_spawn(Some(plan), "verify", "early", None, now()).unwrap();
        let (pinned, _) =
            apply_person_set_gate(Some(recorded), "build", person_gate("build"), now()).unwrap();
        assert!(apply_record_spawn(Some(pinned), "verify", "early", None, now()).is_ok());

        // Through the store: the check passes, the person pins a gate, and the
        // record refuses and writes nothing.
        let (_temp, manager, parent) = store_with_parent().await;
        mutate_plan(&manager, &parent, |_| Ok((three_phase_plan(), ())))
            .await
            .unwrap();
        assert!(validate_spawn_phase(&manager, &parent, "verify")
            .await
            .is_ok());
        person_set_gate(&manager, &parent, "build", person_gate("build"))
            .await
            .unwrap();
        let err = record_spawn(&manager, &parent, "verify", "child", None)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("waits for phase \"build\"'s gate"),
            "{err}"
        );
        let stored = load_plan(&manager, &parent).await.unwrap().unwrap();
        assert!(stored.phase("verify").unwrap().agents.is_empty());
        assert_eq!(stored.revision, 2, "the refused record wrote nothing");
    }

    #[test]
    fn the_agent_cannot_skip_a_gated_phase_but_may_mark_it_blocked() {
        let plan = held_plan();
        let skip_refusal = "Phase \"build\" has a gate, so it closes only through its gate: \
                            complete it with status \"completed\" to ask the gate, or ask the \
                            person, who can pass or skip it in the app or with `biorouter \
                            session gates`. Nothing was changed.";
        for complete in [apply_complete, apply_agent_complete] {
            let err = complete(
                Some(plan.clone()),
                "build",
                PhaseStatus::Skipped,
                None,
                &[],
                now(),
            )
            .unwrap_err();
            assert_eq!(err, skip_refusal);
        }
        let mut workflow = plan.clone();
        workflow
            .phase_mut("build")
            .unwrap()
            .gate
            .as_mut()
            .unwrap()
            .owner = GateOwner::Workflow;
        let err = apply_complete(
            Some(workflow),
            "build",
            PhaseStatus::Skipped,
            None,
            &[],
            now(),
        )
        .unwrap_err();
        assert!(
            err.starts_with("Phase \"build\" has a gate, set by the workflow, so"),
            "{err}"
        );

        // Completing it as completed on a path that asks no gate is refused;
        // it is the gate's to close.
        let err = apply_agent_complete(
            Some(plan.clone()),
            "build",
            PhaseStatus::Completed,
            None,
            &[],
            now(),
        )
        .unwrap_err();
        assert!(err.contains("completing it asks the gate"), "{err}");
        assert_eq!(gate_decision_needed(&plan, "build"), GatePath::Ask);

        let (blocked, note) = apply_agent_complete(
            Some(plan),
            "build",
            PhaseStatus::Blocked,
            Some("Stuck on the grammar."),
            &[],
            now(),
        )
        .unwrap();
        let build = blocked.phase("build").unwrap();
        assert_eq!(build.status, PhaseStatus::Blocked);
        assert!(build.gate_state.is_none(), "no attempt was used");
        assert!(
            note.starts_with(
                "Phase \"build\" is blocked. Its gate was not asked, used no attempt and still \
                 holds the phases after it: the phase completes only through its gate or the \
                 person."
            ),
            "{note}"
        );
        assert!(
            note.contains("Its hand-off is the summary you gave."),
            "{note}"
        );
        assert_eq!(hold_of(&blocked, "verify"), Some("build"));
        let gate = snapshot_of(&blocked, &[]).phases[1].gate.clone().unwrap();
        assert!(gate.needs_person);
        assert!(!gate.blocked, "blocked by the agent, not by its gate");
        // A blocked gated phase can still go through its gate.
        assert_eq!(gate_decision_needed(&blocked, "build"), GatePath::Ask);
        assert!(apply_complete(
            Some(blocked),
            "build",
            PhaseStatus::Completed,
            None,
            &[],
            now()
        )
        .is_ok());
    }

    #[test]
    fn a_person_sets_replaces_and_removes_a_gate_on_an_open_phase_only() {
        let plan = three_phase_plan();
        let gate = validate_person_gate(
            "build",
            &PhaseGateSpec {
                max_attempts: Some(2),
                model: Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b")),
                ..person_spec(&[("cited", "Is every claim cited?")], Some(0.3))
            },
        )
        .unwrap();

        let err = apply_person_set_gate(None, "build", gate.clone(), now()).unwrap_err();
        assert_eq!(err, GateWriteError::NoPlan);
        assert_eq!(
            err.to_string(),
            "This chat has no orchestration plan, so it has no phases to gate."
        );
        let err =
            apply_person_set_gate(Some(plan.clone()), "deploy", gate.clone(), now()).unwrap_err();
        assert!(
            matches!(&err, GateWriteError::NoPhase(s) if s.contains("research, build, verify")),
            "{err:?}"
        );
        let (done, _) = apply_complete(
            Some(plan.clone()),
            "research",
            PhaseStatus::Completed,
            None,
            &[],
            now(),
        )
        .unwrap();
        assert_eq!(
            apply_person_set_gate(Some(done), "research", gate.clone(), now()).unwrap_err(),
            GateWriteError::Closed(
                "Phase \"research\" is already completed; a gate holds only an open phase. \
                 Nothing was changed."
                    .into()
            )
        );
        let mut empty = gate.clone();
        empty.questions.clear();
        assert!(matches!(
            apply_person_set_gate(Some(plan.clone()), "build", empty, now()),
            Err(GateWriteError::Invalid(_))
        ));

        let (set, outcome) =
            apply_person_set_gate(Some(plan.clone()), " build ", gate.clone(), now()).unwrap();
        assert!(outcome.applied);
        assert_eq!(
            outcome.message,
            "Phase \"build\" now has your gate: 1 question, each needing P(yes) of at least 0.30, \
             with 2 attempts. The agent cannot change, remove or skip it, and the phases after \
             it wait until it passes or you pass or skip the phase."
        );
        let stored = set.phase("build").unwrap().gate.clone().unwrap();
        assert_eq!(stored.owner, GateOwner::Person);
        assert_eq!(stored.threshold, 0.3);
        assert_eq!(hold_of(&set, "verify"), Some("build"));
        // The agent cannot change it, and the refusal says whose it is.
        let err = apply_gated_plan(
            Some(set.clone()),
            None,
            None,
            &[
                gated("research", "research", &[], None),
                gated(
                    "build",
                    "implement",
                    &["research"],
                    Some(build_gate_input()),
                ),
                gated("verify", "verify", &[], None),
            ],
        )
        .unwrap_err();
        assert!(
            err.contains("already has a gate, set by the person"),
            "{err}"
        );

        // Replacing a gate its attempts blocked: the record starts over and
        // the phase is back in progress.
        let exhausted = || {
            let mut plan = held_plan();
            let build = plan.phase_mut("build").unwrap();
            build.status = PhaseStatus::Blocked;
            build.gate_state = Some(GateState {
                attempts: 3,
                last_passed: Some(false),
                ..GateState::default()
            });
            plan
        };
        let (replaced, outcome) =
            apply_person_set_gate(Some(exhausted()), "build", gate.clone(), now()).unwrap();
        assert!(
            outcome
                .message
                .contains("It replaces the gate the agent set, and its attempts start over."),
            "{}",
            outcome.message
        );
        assert!(outcome.message.ends_with("It is back in progress."));
        let build = replaced.phase("build").unwrap();
        assert_eq!(build.status, PhaseStatus::InProgress);
        assert!(build.gate_state.is_none());

        // Removing.
        assert_eq!(
            apply_person_remove_gate(Some(set.clone()), "research").unwrap_err(),
            GateWriteError::NoGate("Phase \"research\" has no gate. Nothing was changed.".into())
        );
        let (removed, outcome) = apply_person_remove_gate(Some(set), "build").unwrap();
        assert!(removed.phase("build").unwrap().gate.is_none());
        assert_eq!(
            hold_of(&removed, "verify"),
            None,
            "the phases after it go free"
        );
        assert_eq!(
            outcome.message,
            "Phase \"build\" has no gate now: the agent can complete it without one. The phases \
             after it can go on."
        );
        let (unblocked, outcome) = apply_person_remove_gate(Some(exhausted()), "build").unwrap();
        assert_eq!(
            unblocked.phase("build").unwrap().status,
            PhaseStatus::InProgress
        );
        assert!(
            outcome.message.contains("It is back in progress."),
            "{}",
            outcome.message
        );
    }

    #[test]
    fn a_persons_pass_applies_at_once_or_stands() {
        let plan = with_agents(held_plan(), "build", &["c1"]);
        let running = [state("c1", SubagentRunStatus::Running, None)];
        let done = [state(
            "c1",
            SubagentRunStatus::Completed,
            Some("All tests pass"),
        )];

        let (standing, outcome) = apply_person_gate_decision(
            Some(plan.clone()),
            "build",
            PersonGateChoice::Pass,
            &running,
            now(),
        )
        .unwrap();
        assert!(!outcome.applied);
        assert_eq!(
            outcome.message,
            "Your pass stands: phase \"build\" completes without its gate once its 1 working \
             subagent finishes and the agent completes it."
        );
        let build = standing.phase("build").unwrap();
        assert_eq!(build.status, PhaseStatus::InProgress);
        assert_eq!(
            build.gate_state.as_ref().unwrap().standing(),
            Some(PersonGateAction::Pass)
        );
        assert_eq!(
            snapshot_of(&standing, &[]).phases[1]
                .gate
                .as_ref()
                .unwrap()
                .person_decision,
            Some(PersonGateAction::Pass)
        );
        assert_eq!(
            gate_decision_needed(&standing, "build"),
            GatePath::StandingPass
        );

        // The agent's next complete applies it without asking the gate.
        let (closed, note) = apply_agent_complete(
            Some(standing),
            "build",
            PhaseStatus::Completed,
            None,
            &done,
            now(),
        )
        .unwrap();
        let build = closed.phase("build").unwrap();
        assert_eq!(build.status, PhaseStatus::Completed);
        let state = build.gate_state.as_ref().unwrap();
        assert_eq!(state.closed_by_person, Some(PersonGateAction::Pass));
        assert_eq!(state.person, None);
        assert_eq!(state.attempts, 0, "the gate was not asked");
        assert!(
            note.ends_with(
                "The person passed this phase's gate, so it completed without asking it."
            ),
            "{note}"
        );
        assert_eq!(hold_of(&closed, "verify"), None);

        // With nothing working, it applies at once.
        let (applied, outcome) = apply_person_gate_decision(
            Some(plan.clone()),
            "build",
            PersonGateChoice::Pass,
            &done,
            now(),
        )
        .unwrap();
        assert!(outcome.applied);
        assert_eq!(
            outcome.message,
            "Phase \"build\" is completed: you passed its gate. The phases after it can go on."
        );
        assert_eq!(
            applied.phase("build").unwrap().status,
            PhaseStatus::Completed
        );
        assert_eq!(
            snapshot_of(&applied, &[]).phases[1]
                .gate
                .as_ref()
                .unwrap()
                .person_decision,
            Some(PersonGateAction::Pass),
            "the snapshot keeps the decision that closed it"
        );

        // A child recorded after the states were read stands the decision too.
        let (unchecked, outcome) = apply_person_gate_decision(
            Some(plan.clone()),
            "build",
            PersonGateChoice::Pass,
            &[],
            now(),
        )
        .unwrap();
        assert!(!outcome.applied);
        assert_eq!(
            unchecked.phase("build").unwrap().status,
            PhaseStatus::InProgress
        );

        // An earlier gate does not hold the person.
        let mut both = held_plan();
        both.phase_mut("research").unwrap().gate = Some(person_gate("research"));
        assert_eq!(hold_of(&both, "build"), Some("research"));
        let (passed, outcome) =
            apply_person_gate_decision(Some(both), "build", PersonGateChoice::Pass, &[], now())
                .unwrap();
        assert!(outcome.applied);
        assert!(
            outcome
                .message
                .ends_with("The phases after it still wait for phase \"research\"'s gate."),
            "{}",
            outcome.message
        );
        assert_eq!(
            passed.phase("build").unwrap().status,
            PhaseStatus::Completed
        );

        // Refusals.
        assert_eq!(
            apply_person_gate_decision(
                Some(three_phase_plan()),
                "build",
                PersonGateChoice::Pass,
                &[],
                now()
            )
            .unwrap_err(),
            GateWriteError::NoGate("Phase \"build\" has no gate. Nothing was changed.".into())
        );
        assert!(matches!(
            apply_person_gate_decision(Some(applied), "build", PersonGateChoice::Skip, &[], now()),
            Err(GateWriteError::Closed(_))
        ));
    }

    #[test]
    fn a_persons_skip_closes_a_gated_phase_and_a_standing_skip_admits_the_agents() {
        let plan = with_agents(held_plan(), "build", &["c1", "c2"]);
        let running = [
            state("c1", SubagentRunStatus::Running, None),
            state("c2", SubagentRunStatus::Queued, None),
        ];
        let done = [
            state("c1", SubagentRunStatus::Completed, Some("half done")),
            state("c2", SubagentRunStatus::Failed, None),
        ];

        let (skipped, outcome) = apply_person_gate_decision(
            Some(plan.clone()),
            "build",
            PersonGateChoice::Skip,
            &done,
            now(),
        )
        .unwrap();
        assert!(outcome.applied);
        assert_eq!(
            outcome.message,
            "Phase \"build\" is skipped: you skipped it, so its gate was not asked. The phases \
             after it can go on."
        );
        let build = skipped.phase("build").unwrap();
        assert_eq!(build.status, PhaseStatus::Skipped);
        assert_eq!(
            build.gate_state.as_ref().unwrap().closed_by_person,
            Some(PersonGateAction::Skip)
        );
        assert!(build.handoff.is_none(), "a skip composes no hand-off");
        assert_eq!(hold_of(&skipped, "verify"), None);

        let (standing, outcome) = apply_person_gate_decision(
            Some(plan),
            "build",
            PersonGateChoice::Skip,
            &running,
            now(),
        )
        .unwrap();
        assert!(!outcome.applied);
        assert_eq!(
            outcome.message,
            "Your skip stands: phase \"build\" is skipped without its gate once its 2 working \
             subagents finish and the agent completes it."
        );
        // While the skip stands, the agent's "completed" is refused before
        // anything is asked, and its skip is admitted.
        let refusal = "The person skipped phase \"build\", so it closes only as skipped: \
                       complete it with status \"skipped\" once its subagents finish. Nothing \
                       was changed.";
        assert_eq!(
            gate_decision_needed(&standing, "build"),
            GatePath::Held(refusal.to_string())
        );
        assert_eq!(
            apply_complete(
                Some(standing.clone()),
                "build",
                PhaseStatus::Completed,
                None,
                &done,
                now()
            )
            .unwrap_err(),
            refusal
        );
        let (closed, note) = apply_agent_complete(
            Some(standing),
            "build",
            PhaseStatus::Skipped,
            None,
            &done,
            now(),
        )
        .unwrap();
        assert_eq!(closed.phase("build").unwrap().status, PhaseStatus::Skipped);
        assert_eq!(
            closed
                .phase("build")
                .unwrap()
                .gate_state
                .as_ref()
                .unwrap()
                .closed_by_person,
            Some(PersonGateAction::Skip)
        );
        assert!(
            note.ends_with("The person skipped this phase, so it closed without its gate."),
            "{note}"
        );
    }

    #[test]
    fn retry_gives_an_exhausted_gate_its_attempts_again() {
        let mut plan = gated_plan();
        let gate = stored_gate(&plan);
        let fail = GateVerdict::Failed(answers(&[("tests_pass", 0.42), ("scope_met", 0.91)], 0.8));
        for _ in 0..3 {
            plan = apply_gate_verdict(Some(plan), "build", &gate, &fail, None, &[], now())
                .unwrap()
                .0;
        }
        let build = plan.phase("build").unwrap();
        assert_eq!(build.status, PhaseStatus::Blocked);
        let refusal = refuse_exhausted_gate(build).unwrap();
        assert!(
            refusal.contains("Only they can go on from here"),
            "{refusal}"
        );
        assert!(
            !refusal.contains("skipped"),
            "the agent is not told to skip: {refusal}"
        );

        let (retried, outcome) =
            apply_person_gate_decision(Some(plan), "build", PersonGateChoice::Retry, &[], now())
                .unwrap();
        assert!(outcome.applied);
        assert_eq!(
            outcome.message,
            "Phase \"build\"'s gate has its 3 attempts again. The agent can complete the phase \
             once more to ask it."
        );
        let build = retried.phase("build").unwrap();
        assert_eq!(build.status, PhaseStatus::InProgress);
        let state = build.gate_state.clone().unwrap();
        assert_eq!(
            (
                state.attempts,
                state.consecutive_errors,
                state.last_passed,
                state.last_error,
                state.retries_granted
            ),
            (0, 0, None, None, 1)
        );
        assert!(refuse_exhausted_gate(build).is_none());
        let snap = snapshot_of(&retried, &[]).phases[1].gate.clone().unwrap();
        assert_eq!(snap.retries_granted, 1);
        assert!(!snap.blocked && !snap.needs_person);

        // A gate blocked by errors in a row is reopened too, and a standing
        // decision is dropped.
        let error = GateVerdict::Error("the decision server failed".into());
        let mut errored = gated_plan();
        for _ in 0..2 {
            errored = apply_gate_verdict(Some(errored), "build", &gate, &error, None, &[], now())
                .unwrap()
                .0;
        }
        errored
            .phase_mut("build")
            .unwrap()
            .gate_state
            .as_mut()
            .unwrap()
            .person = Some(PersonGateDecision {
            decision: PersonGateAction::Pass,
            at: now(),
        });
        let (retried, _) =
            apply_person_gate_decision(Some(errored), "build", PersonGateChoice::Retry, &[], now())
                .unwrap();
        let build = retried.phase("build").unwrap();
        assert_eq!(build.status, PhaseStatus::InProgress);
        assert_eq!(build.gate_state.as_ref().unwrap().person, None);
        assert_eq!(build.gate_state.as_ref().unwrap().consecutive_errors, 0);
    }

    /// A gate that failed every attempt keeps its attempts after the person
    /// passes or skips the phase, but the closed phase is blocked on nothing
    /// and offers the person nothing more to do.
    #[test]
    fn a_phase_the_person_closed_after_its_gate_failed_every_attempt_needs_no_one() {
        let mut plan = gated_plan();
        let gate = stored_gate(&plan);
        let fail = GateVerdict::Failed(answers(&[("tests_pass", 0.42), ("scope_met", 0.91)], 0.8));
        for _ in 0..3 {
            plan = apply_gate_verdict(Some(plan), "build", &gate, &fail, None, &[], now())
                .unwrap()
                .0;
        }
        let exhausted = snapshot_of(&plan, &[]).phases[1].gate.clone().unwrap();
        assert!(exhausted.blocked && exhausted.needs_person);

        for (choice, decision, status) in [
            (
                PersonGateChoice::Pass,
                PersonGateAction::Pass,
                PhaseStatus::Completed,
            ),
            (
                PersonGateChoice::Skip,
                PersonGateAction::Skip,
                PhaseStatus::Skipped,
            ),
        ] {
            let (closed, outcome) =
                apply_person_gate_decision(Some(plan.clone()), "build", choice, &[], now())
                    .unwrap();
            assert!(outcome.applied, "{}", outcome.message);
            assert_eq!(closed.phase("build").unwrap().status, status);
            let snapshot = snapshot_of(&closed, &[]);
            let gate = snapshot.phases[1].gate.clone().unwrap();
            assert_eq!(gate.attempts, 3, "the attempts it used are kept");
            assert_eq!(gate.person_decision, Some(decision));
            assert!(
                !gate.needs_person,
                "a closed phase needs no one: {status:?}"
            );
            assert!(
                !gate.blocked,
                "a closed phase is blocked on nothing: {status:?}"
            );
            let words = gate_words(&gate);
            assert!(!words.contains("blocked"), "{words}");
            let text = render_snapshot_text(&snapshot);
            assert!(!text.contains("only the person"), "{text}");
            assert!(!render_moim(&snapshot).contains("blocked"), "{status:?}");
        }
    }

    /// The plan text says only the person can move a blocked gated phase on
    /// exactly when the agent cannot: its gate failed every attempt. After an
    /// error block or the agent's own block, completing the phase asks the
    /// gate again, and the text says so.
    #[test]
    fn the_plan_text_leaves_a_blocked_gate_to_the_person_only_when_the_agent_has_no_way_on() {
        let only_the_person = "only the person can move this phase on now";
        // A phase's lines in the plan text: its numbered line and the
        // indented ones under it.
        let line_of = |plan: &OrchestrationPlan, phase: &str| {
            let text = render_snapshot_text(&snapshot_of(plan, &[]));
            let mut lines = text
                .lines()
                .skip_while(|line| !line.contains(&format!(". {phase} [")));
            let mut block = lines
                .next()
                .unwrap_or_else(|| panic!("no {phase} in {text}"))
                .to_string();
            for line in lines.take_while(|line| line.starts_with("   ")) {
                block.push('\n');
                block.push_str(line);
            }
            block
        };

        // The agent marked the gated phase blocked: its gate can still be asked.
        let (self_blocked, _) = apply_agent_complete(
            Some(held_plan()),
            "build",
            PhaseStatus::Blocked,
            None,
            &[],
            now(),
        )
        .unwrap();
        assert!(
            snapshot_of(&self_blocked, &[]).phases[1]
                .gate
                .as_ref()
                .unwrap()
                .needs_person
        );
        assert_eq!(gate_decision_needed(&self_blocked, "build"), GatePath::Ask);
        assert!(refuse_exhausted_gate(self_blocked.phase("build").unwrap()).is_none());
        let line = line_of(&self_blocked, "build");
        assert!(!line.contains(only_the_person), "{line}");
        assert!(
            line.contains(
                "marked blocked, so its gate was not asked: complete the phase with status \
                 \"completed\" to ask it, or the person can change, remove, pass or skip its \
                 gate"
            ),
            "{line}"
        );

        // Blocked by errors in a row: asking again is the agent's to try once a
        // decision model is reachable, as the error note said.
        let mut errored = gated_plan();
        let gate = stored_gate(&errored);
        let error = GateVerdict::Error("the decision server failed".into());
        let mut note = String::new();
        for _ in 0..GATE_ERRORS_TO_BLOCK {
            let (next, said, _) =
                apply_gate_verdict(Some(errored), "build", &gate, &error, None, &[], now())
                    .unwrap();
            errored = next;
            note = said;
        }
        assert!(
            note.contains("you can complete the phase again to ask it"),
            "{note}"
        );
        assert_eq!(gate_decision_needed(&errored, "build"), GatePath::Ask);
        assert!(refuse_exhausted_gate(errored.phase("build").unwrap()).is_none());
        let line = line_of(&errored, "build");
        assert!(!line.contains(only_the_person), "{line}");
        assert!(
            line.contains(
                "blocked because its gate could not be evaluated 2 times in a row: complete the \
                 phase again to ask it once the model that judges it can be reached, or \
                 the person can retry, change, remove, pass or skip its gate"
            ),
            "{line}"
        );

        // Every attempt failed: only the person can go on, and the agent's
        // completion is refused.
        let mut exhausted = gated_plan();
        let fail = GateVerdict::Failed(answers(&[("tests_pass", 0.42), ("scope_met", 0.91)], 0.8));
        for _ in 0..3 {
            exhausted =
                apply_gate_verdict(Some(exhausted), "build", &gate, &fail, None, &[], now())
                    .unwrap()
                    .0;
        }
        assert!(refuse_exhausted_gate(exhausted.phase("build").unwrap()).is_some());
        let line = line_of(&exhausted, "build");
        assert!(line.contains(only_the_person), "{line}");

        // A blocked gated phase an earlier gate holds says it waits for that
        // gate first.
        let mut two = held_plan();
        two.phase_mut("verify").unwrap().gate = Some(stored_gate(&two));
        let (held_blocked, _) =
            apply_agent_complete(Some(two), "verify", PhaseStatus::Blocked, None, &[], now())
                .unwrap();
        let line = line_of(&held_blocked, "verify");
        assert!(line.contains("waits for build's gate"), "{line}");
        assert!(
            line.contains("to ask it once build's gate is through, or the person"),
            "{line}"
        );
    }

    /// The ways past a full or finished plan name its gated open phases apart:
    /// the agent may skip only the others.
    #[test]
    fn the_ways_past_a_full_plan_name_gated_open_phases_apart() {
        let mut full = held_plan();
        let ids: Vec<String> = (0..MAX_PLAN_AGENTS).map(|i| format!("k{i}")).collect();
        for id in &ids {
            full = apply_record_spawn(Some(full), "research", id, None, now()).unwrap();
        }
        let gated_words = "Phase build has a gate, so it closes only through its gate or the \
                           person, who can pass or skip it in the app or with `biorouter session \
                           gates`.";

        // The plan limit names every open phase, held ones too, since
        // fresh:true needs them all closed.
        let err = check_spawn_phase(Some(&full), "research").unwrap_err();
        assert!(
            err.contains("close its open phases (research, build, verify)"),
            "{err}"
        );
        assert!(
            err.contains(
                "Close research, verify with action:\"complete\" (status:\"skipped\" for work \
                 you will not do)."
            ),
            "{err}"
        );
        assert!(err.contains(gated_words), "{err}");
        assert!(err.ends_with("Nothing was created."), "{err}");

        let fresh = |plan: &OrchestrationPlan| {
            apply_fresh_gated_plan(
                Some(plan.clone()),
                None,
                None,
                &[gated("next", "plan", &[], None)],
            )
            .unwrap_err()
        };
        let err = fresh(&full);
        assert!(
            err.contains("still open: research, build, verify."),
            "{err}"
        );
        assert!(err.contains("Close research, verify with action"), "{err}");
        assert!(err.contains(gated_words), "{err}");

        // The advice holds: the agent's skips close the ungated phases, and
        // only build is left, which its skip cannot close.
        let done: Vec<PhaseAgentState> = ids
            .iter()
            .map(|id| state(id, SubagentRunStatus::Completed, None))
            .collect();
        let none: &[PhaseAgentState] = &[];
        let mut rest = full;
        for (phase, checked) in [("research", done.as_slice()), ("verify", none)] {
            rest = apply_agent_complete(
                Some(rest),
                phase,
                PhaseStatus::Skipped,
                None,
                checked,
                now(),
            )
            .unwrap()
            .0;
        }
        let err = fresh(&rest);
        assert!(err.contains("still open: build."), "{err}");
        assert!(!err.contains("Close"), "nothing left to skip: {err}");
        assert!(err.contains(gated_words), "{err}");
        assert!(apply_agent_complete(
            Some(rest.clone()),
            "build",
            PhaseStatus::Skipped,
            None,
            &[],
            now()
        )
        .is_err());

        // Two gated phases, in the plural.
        let mut two = held_plan();
        two.phase_mut("verify").unwrap().gate = Some(stored_gate(&two));
        let err = fresh(&two);
        assert!(
            err.contains(
                "Phases build, verify have gates, so they close only through their gates or the \
                 person, who can pass or skip them"
            ),
            "{err}"
        );
        assert!(err.contains("Close research with action"), "{err}");
    }

    /// The person may set a gate on an earlier phase while a later phase's
    /// gate is being evaluated: a pass is then not recorded, and the phase
    /// stays open.
    #[test]
    fn a_verdict_is_refused_when_an_earlier_gate_now_holds_the_phase() {
        let plan = gated_plan();
        let gate = stored_gate(&plan);
        let (pinned, _) =
            apply_person_set_gate(Some(plan), "research", person_gate("research"), now()).unwrap();
        let pass = GateVerdict::Passed(answers(&[("tests_pass", 0.9), ("scope_met", 0.9)], 0.8));
        let err = apply_gate_verdict(
            Some(pinned.clone()),
            "build",
            &gate,
            &pass,
            None,
            &[],
            now(),
        )
        .unwrap_err();
        assert_eq!(
            err,
            "Phase \"build\" now waits for phase \"research\"'s gate, set while its own gate was \
             evaluated. The verdict was not recorded; complete it again once \"research\" is \
             through."
        );
        // A failure is still recorded: the phase stays open either way.
        let fail = GateVerdict::Failed(answers(&[("tests_pass", 0.2), ("scope_met", 0.9)], 0.8));
        let (next, _, closed) =
            apply_gate_verdict(Some(pinned), "build", &gate, &fail, None, &[], now()).unwrap();
        assert!(!closed);
        assert_eq!(
            next.phase("build")
                .unwrap()
                .gate_state
                .as_ref()
                .unwrap()
                .attempts,
            1
        );
    }

    /// A plan the previous build stored, before gates had owners or a person's
    /// record and phases had monitors, reads with the agent as every gate's
    /// owner.
    #[test]
    fn a_plan_stored_before_gate_owners_parses_as_agent_gates() {
        let stored = serde_json::json!({
            "title": "Ship the parser",
            "revision": 4,
            "phases": [{
                "id": "build",
                "title": "Build",
                "kind": "implementation",
                "status": "in_progress",
                "depends_on": [],
                "agents": [],
                "gate": {
                    "questions": {
                        "tests_pass": {
                            "instructions": "Do the tests pass?",
                            "kind": {"binary": {}}
                        }
                    },
                    "threshold": 0.3,
                    "model": null,
                    "maxAttempts": 3,
                    "allowUncalibrated": false
                },
                "gate_state": {
                    "attempts": 1,
                    "consecutive_errors": 0,
                    "last_passed": false,
                    "last_min_p": 0.25
                }
            }, {
                "id": "ship",
                "title": "Ship",
                "kind": "execution",
                "status": "pending"
            }]
        });
        let plan = parse_stored(&stored).unwrap();
        let build = plan.phase("build").unwrap();
        let gate = build.gate.as_ref().unwrap();
        assert_eq!(gate.owner, GateOwner::Agent);
        assert_eq!(gate.threshold, 0.3, "a stored gate keeps its terms");
        let state = build.gate_state.as_ref().unwrap();
        assert_eq!(state.person, None);
        assert_eq!(state.retries_granted, 0);
        assert_eq!(state.closed_by_person, None);
        assert!(build.monitors.is_empty());
        // And it binds: the agent cannot skip it, and it holds "ship".
        assert_eq!(hold_of(&plan, "ship"), Some("build"));
        assert!(apply_complete(
            Some(plan.clone()),
            "build",
            PhaseStatus::Skipped,
            None,
            &[],
            now()
        )
        .is_err());
        // Written back, a record with no new state carries no new keys.
        let written = plan_record(&plan);
        let state = &written["phases"][0]["gate_state"];
        for key in ["person", "retries_granted", "closed_by_person"] {
            assert!(state.get(key).is_none(), "{key}: {state}");
        }
        assert!(written["phases"][0].get("monitors").is_none());
    }

    #[test]
    fn the_snapshot_carries_owner_holds_and_held_by() {
        let plan = apply_gated_plan(
            None,
            Some("Ship the parser"),
            None,
            &[
                gated("research", "research", &[], None),
                gated(
                    "build",
                    "implement",
                    &["research"],
                    Some(PhaseGateInput {
                        model: Some("ollama_decision/tev1:0.8b".into()),
                        allow_uncalibrated: Some(true),
                        ..build_gate_input()
                    }),
                ),
                gated("verify", "verify", &[], None),
            ],
        )
        .unwrap();
        let json = serde_json::to_value(snapshot_of(&plan, &[])).unwrap();
        let build = &json["phases"][1];
        let gate = &build["gate"];
        assert_eq!(gate["owner"], "agent");
        assert_eq!(
            gate["questions"],
            serde_json::json!({
                "tests_pass": "Do the results show every new test passing?",
                "scope_met": "Did the phase deliver every item in its goal?",
            })
        );
        assert_eq!(gate["model"], "ollama_decision/tev1:0.8b");
        assert_eq!(gate["allowUncalibrated"], true);
        assert_eq!(gate["lastError"], serde_json::Value::Null);
        assert_eq!(gate["personDecision"], serde_json::Value::Null);
        assert_eq!(gate["retriesGranted"], 0);
        assert_eq!(gate["needsPerson"], false);
        assert_eq!(gate["holds"], serde_json::json!(["verify"]));
        assert_eq!(build["heldBy"], serde_json::Value::Null);
        assert_eq!(json["phases"][2]["heldBy"], "build");

        // An error-blocked gate needs the person and says why.
        let gate_terms = stored_gate(&plan);
        let error = GateVerdict::Error("the decision server failed".into());
        let mut errored = plan.clone();
        for _ in 0..2 {
            errored = apply_gate_verdict(
                Some(errored),
                "build",
                &gate_terms,
                &error,
                None,
                &[],
                now(),
            )
            .unwrap()
            .0;
        }
        let json = serde_json::to_value(snapshot_of(&errored, &[])).unwrap();
        assert_eq!(json["phases"][1]["gate"]["needsPerson"], true);
        assert_eq!(
            json["phases"][1]["gate"]["lastError"],
            "the decision server failed"
        );

        // The person's gate and standing decision.
        let (owned, _) =
            apply_person_set_gate(Some(plan), "build", person_gate("build"), now()).unwrap();
        let owned = with_agents(owned, "build", &["c1"]);
        let (standing, _) = apply_person_gate_decision(
            Some(owned),
            "build",
            PersonGateChoice::Skip,
            &[state("c1", SubagentRunStatus::Running, None)],
            now(),
        )
        .unwrap();
        let json = serde_json::to_value(snapshot_of(&standing, &[])).unwrap();
        assert_eq!(json["phases"][1]["gate"]["owner"], "person");
        assert_eq!(json["phases"][1]["gate"]["personDecision"], "skip");

        // A snapshot from an older daemon, without the new keys, still reads.
        let older: PhaseSnapshot = serde_json::from_value(serde_json::json!({
            "id": "build", "title": "Build", "goal": null, "kind": "implementation",
            "status": "in_progress", "dependsOn": [], "agentIds": [],
            "counts": {"total": 0, "done": 0, "running": 0, "blocked": 0, "failed": 0, "quiet": 0},
            "handoff": null, "startedAt": null, "finishedAt": null,
            "gate": {"threshold": 0.8, "questionCount": 1, "attempts": 0, "maxAttempts": 3,
                     "lastPassed": null, "lastMinP": null, "blocked": false}
        }))
        .unwrap();
        assert_eq!(older.held_by, None);
        let older_gate = older.gate.unwrap();
        assert_eq!(older_gate.owner, GateOwner::Agent);
        assert_eq!(older_gate.model_adapter, None);
    }

    /// The desktop's editor rebuilds a gate from its snapshot and writes the
    /// whole gate back, so the snapshot says which adapter the gate's model is
    /// asked through. Without it an edit to one question quietly moved a
    /// `logprob` gate onto the model's default adapter.
    #[test]
    fn the_snapshot_says_which_adapter_the_gates_model_is_asked_through() {
        let gate_for = |model: Option<DecisionModelRef>| {
            let mut spec = person_spec(&[("cited", "Is every claim cited?")], None);
            spec.model = model;
            let (plan, _) = apply_person_set_gate(
                Some(gated_plan()),
                "build",
                validate_person_gate("build", &spec).unwrap(),
                now(),
            )
            .unwrap();
            serde_json::to_value(snapshot_of(&plan, &[])).unwrap()["phases"][1]["gate"].clone()
        };

        let named = gate_for(Some(
            DecisionModelRef::new("ollama_decision", "tev1:0.8b")
                .with_adapter(DecisionAdapter::Logprob),
        ));
        assert_eq!(named["model"], "ollama_decision/tev1:0.8b");
        assert_eq!(named["modelAdapter"], "logprob");
        // A client that reads the snapshot back gets the same adapter.
        let back: PhaseGateSnapshot = serde_json::from_value(named).unwrap();
        assert_eq!(back.model_adapter, Some(DecisionAdapter::Logprob));

        // A model named without an adapter, and no model at all, say none.
        let plain = gate_for(Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b")));
        assert_eq!(plain["model"], "ollama_decision/tev1:0.8b");
        assert_eq!(plain["modelAdapter"], serde_json::Value::Null);
        let unnamed = gate_for(None);
        assert_eq!(unnamed["model"], serde_json::Value::Null);
        assert_eq!(unnamed["modelAdapter"], serde_json::Value::Null);
    }

    /// Live QA M2-4: a gate that names its decision model says which, as a
    /// person reads it, so the plan card shows which model will read the
    /// chat. One that names none says nothing more (it runs on the default).
    #[test]
    fn the_plan_text_names_a_gates_decision_model() {
        let plan = held_plan();
        let snapshot = snapshot_of(&plan, &[]);
        let mut gate = snapshot
            .phases
            .iter()
            .find_map(|phase| phase.gate.clone())
            .expect("the held plan has a gate");
        gate.model = None;
        assert!(
            gate_words(&gate).starts_with("2 questions at P(yes) >= 0.80, 0 of 3 attempts used"),
            "{}",
            gate_words(&gate)
        );
        gate.model = Some("baseten_decision/joey-1".to_string());
        assert!(
            gate_words(&gate).starts_with(
                "2 questions at P(yes) >= 0.80 with joey-1 · Baseten, 0 of 3 attempts used"
            ),
            "{}",
            gate_words(&gate)
        );
        // Stored by its slug, it is still named by its alias.
        gate.model = Some("baseten_decision/decision-models/baseten-joey-2b-latest".to_string());
        assert!(
            gate_words(&gate).starts_with(
                "2 questions at P(yes) >= 0.80 with joey-1 · Baseten, 0 of 3 attempts used"
            ),
            "{}",
            gate_words(&gate)
        );
        gate.model = Some("test_decision/tev1".to_string());
        assert!(
            gate_words(&gate).starts_with(
                "2 questions at P(yes) >= 0.80 with test_decision/tev1, 0 of 3 attempts used"
            ),
            "{}",
            gate_words(&gate)
        );
    }

    #[test]
    fn the_moim_and_plan_text_say_who_set_a_gate_and_what_waits() {
        let plan = held_plan();
        let snapshot = snapshot_of(&plan, &[]);
        let text = render_snapshot_text(&snapshot);
        assert!(
            text.contains(
                "gate: 2 questions at P(yes) >= 0.80, 0 of 3 attempts used, set by the agent, \
                 holds verify"
            ),
            "{text}"
        );
        assert!(
            text.contains("3. verify [verification] \"verify title\": pending, 0/0 done; waits for build's gate"),
            "{text}"
        );
        let moim = render_moim(&snapshot);
        assert!(
            moim.contains("- build (implementation): pending, 0/0 done, gated (attempt 0 of 3)\n"),
            "{moim}"
        );
        assert!(
            moim.contains("- verify (verification): pending, 0/0 done, waits for build's gate"),
            "{moim}"
        );

        // The person's gate, with their pass standing.
        let (owned, _) =
            apply_person_set_gate(Some(plan.clone()), "build", person_gate("build"), now())
                .unwrap();
        let owned = with_agents(owned, "build", &["c1"]);
        let (standing, _) = apply_person_gate_decision(
            Some(owned),
            "build",
            PersonGateChoice::Pass,
            &[state("c1", SubagentRunStatus::Running, None)],
            now(),
        )
        .unwrap();
        let snapshot = snapshot_of(&standing, &[row("c1", SubagentRunStatus::Running, None)]);
        let text = render_snapshot_text(&snapshot);
        assert!(
            text.contains("set by the person, passed by the person, holds verify"),
            "{text}"
        );
        assert!(text.contains("the person passed this gate"), "{text}");
        let moim = render_moim(&snapshot);
        assert!(
            moim.contains("gated (attempt 0 of 3, set by the person, passed by the person)"),
            "{moim}"
        );

        // A workflow's gate, and a gate only the person can move on.
        let mut workflow = plan;
        {
            let build = workflow.phase_mut("build").unwrap();
            build.gate.as_mut().unwrap().owner = GateOwner::Workflow;
            build.status = PhaseStatus::Blocked;
            build.gate_state = Some(GateState {
                attempts: 3,
                last_passed: Some(false),
                last_min_p: Some(0.4),
                ..GateState::default()
            });
        }
        let snapshot = snapshot_of(&workflow, &[]);
        let text = render_snapshot_text(&snapshot);
        assert!(
            text.contains("last not passed (lowest P(yes) 0.40), blocked, set by the workflow"),
            "{text}"
        );
        assert!(
            text.contains("only the person can move this phase on now"),
            "{text}"
        );
        assert!(
            render_moim(&snapshot).contains("gated (attempt 3 of 3, blocked, set by the workflow)"),
            "{}",
            render_moim(&snapshot)
        );

        // The MOIM keeps its cap with every phase gated and held.
        let many: Vec<OrchestratePhaseInput> = (0..MAX_PHASES)
            .map(|i| OrchestratePhaseInput {
                phase: PlanPhaseInput {
                    title: "t".repeat(MAX_TITLE_CHARS),
                    ..input(&format!("phase-with-a-long-id-{i:02}"), "other", &[])
                },
                gate: Some(build_gate_input()),
                monitors: None,
            })
            .collect();
        let wide = apply_workflow_plan("wide", None, many, now()).unwrap();
        assert!(render_moim(&snapshot_of(&wide, &[])).chars().count() <= 2_000);
    }

    #[test]
    fn phase_monitors_are_validated_kept_on_revision_and_cleared_by_an_empty_list() {
        let work = |monitors: Option<Vec<MonitorSpec>>| OrchestratePhaseInput {
            phase: input("work", "execution", &[]),
            gate: None,
            monitors,
        };
        let plan = apply_gated_plan(
            None,
            Some("t"),
            None,
            &[work(Some(vec![monitor_spec("Off task")]))],
        )
        .unwrap();
        assert_eq!(
            phase_monitors_for(&plan, "work"),
            vec![monitor_spec("Off task")]
        );
        assert_eq!(
            phase_monitors_for(&plan, " work "),
            vec![monitor_spec("Off task")]
        );
        assert!(phase_monitors_for(&plan, "nope").is_empty());
        let stored = plan_record(&plan);
        assert_eq!(stored["phases"][0]["monitors"][0]["name"], "Off task");
        assert_eq!(
            parse_stored(&stored).unwrap(),
            plan,
            "the monitors round-trip"
        );

        // Validated when the plan is written.
        let five: Vec<MonitorSpec> = (0..5).map(|i| monitor_spec(&format!("m{i}"))).collect();
        let mut zero = monitor_spec("Never");
        zero.threshold = 0.0;
        let mut twin = monitor_spec("Twin");
        twin.id = Some("off-task".into());
        for (monitors, expected) in [
            (five, "Phase \"work\" may carry at most 4 monitors, not 5."),
            (
                vec![zero],
                "Phase \"work\"'s monitors: Monitor \"Never\": the threshold",
            ),
            (
                vec![monitor_spec("Off task"), twin],
                "Phase \"work\" names monitor id `off-task` twice",
            ),
        ] {
            let err = apply_gated_plan(None, Some("t"), None, &[work(Some(monitors))]).unwrap_err();
            assert!(
                err.starts_with(expected),
                "expected {expected:?}, got {err:?}"
            );
        }
        let mut bad_model = monitor_spec("Bad model");
        bad_model.model = Some("tev1".into());
        assert!(apply_gated_plan(None, Some("t"), None, &[work(Some(vec![bad_model]))]).is_err());

        // A revision that leaves them out keeps them; [] clears them.
        let kept = apply_gated_plan(Some(plan.clone()), None, None, &[work(None)]).unwrap();
        assert_eq!(phase_monitors_for(&kept, "work").len(), 1);
        let busy = with_agents(plan.clone(), "work", &["c1"]);
        let cleared =
            apply_gated_plan(Some(busy.clone()), None, None, &[work(Some(vec![]))]).unwrap();
        assert!(phase_monitors_for(&cleared, "work").is_empty());
        assert!(plan_record(&cleared)["phases"][0].get("monitors").is_none());

        // The reply says so only when children already run under the old ones.
        assert_eq!(
            revision_monitor_note(Some(&busy), &[work(Some(vec![]))]),
            Some(PHASE_MONITORS_NOT_RETROACTIVE)
        );
        assert_eq!(revision_monitor_note(Some(&busy), &[work(None)]), None);
        assert_eq!(
            revision_monitor_note(Some(&busy), &[work(Some(vec![monitor_spec("Off task")]))]),
            None,
            "the same monitors are no change"
        );
        assert_eq!(
            revision_monitor_note(Some(&plan), &[work(Some(vec![]))]),
            None
        );
        assert_eq!(revision_monitor_note(None, &[work(Some(vec![]))]), None);
    }

    #[test]
    fn a_workflow_plan_has_workflow_owned_gates_with_no_half_floor() {
        let phases = vec![
            gated("search", "research", &[], None),
            gated(
                "screen",
                "review",
                &["search"],
                Some(PhaseGateInput {
                    threshold: Some(0.2),
                    model: Some("ollama_decision/tev1:0.8b".into()),
                    ..gate_input(&[("enough", "Did the screening keep at least 20 papers?")])
                }),
            ),
        ];
        assert!(validate_workflow_phases("Literature review", None, &phases).is_ok());
        let plan =
            apply_workflow_plan("Literature review", Some("Twenty papers"), phases, now()).unwrap();
        assert_eq!(plan.revision, 1);
        assert_eq!(plan.goal.as_deref(), Some("Twenty papers"));
        let gate = plan.phase("screen").unwrap().gate.clone().unwrap();
        assert_eq!((gate.owner, gate.threshold), (GateOwner::Workflow, 0.2));
        let record = plan_record(&plan);
        assert_eq!(record["phases"][1]["gate"]["owner"], "workflow");
        assert_eq!(parse_stored(&record).unwrap(), plan);
        assert_eq!(hold_of(&plan, "search"), None);

        // The agent is bound by it.
        let err = apply_gated_plan(
            Some(plan.clone()),
            None,
            None,
            &[
                gated("search", "research", &[], None),
                gated(
                    "screen",
                    "review",
                    &["search"],
                    Some(gate_input(&[("q", "Done?")])),
                ),
            ],
        )
        .unwrap_err();
        assert!(
            err.contains("already has a gate, set by the workflow"),
            "{err}"
        );

        // A dry run refuses what a write would.
        let thirteen: Vec<OrchestratePhaseInput> = (0..13)
            .map(|i| gated(&format!("p{i}"), "other", &[], None))
            .collect();
        for (phases, expected) in [
            (thirteen, "at most 12 phases"),
            (
                vec![
                    gated("a", "plan", &["b"], None),
                    gated("b", "plan", &[], None),
                ],
                "not an earlier phase",
            ),
            (
                vec![gated(
                    "a",
                    "plan",
                    &[],
                    Some(PhaseGateInput {
                        threshold: Some(1.5),
                        ..gate_input(&[("q", "Done?")])
                    }),
                )],
                "from 0 to 1",
            ),
        ] {
            let err = validate_workflow_phases("t", None, &phases).unwrap_err();
            assert!(err.contains(expected), "expected {expected:?} in {err:?}");
        }
        assert!(
            validate_workflow_phases(" ", None, &[gated("a", "plan", &[], None)])
                .unwrap_err()
                .contains("needs a title")
        );
    }

    #[tokio::test]
    async fn the_declared_skeleton_carries_no_state() {
        let (_temp, manager, parent) = store_with_parent().await;
        assert!(declared_skeleton(&manager, &parent).await.is_none());

        let mut plan = apply_gated_plan(
            None,
            Some("Ship the parser"),
            Some("A parser with tests"),
            &[
                OrchestratePhaseInput {
                    phase: PlanPhaseInput {
                        goal: Some("Read the old code".into()),
                        ..input("research", "research", &[])
                    },
                    gate: None,
                    monitors: Some(vec![monitor_spec("Off task")]),
                },
                gated(
                    "build",
                    "implement",
                    &["research"],
                    Some(PhaseGateInput {
                        model: Some("ollama_decision/tev1:0.8b".into()),
                        max_attempts: Some(4),
                        ..build_gate_input()
                    }),
                ),
            ],
        )
        .unwrap();
        plan = with_agents(plan, "research", &["c1"]);
        plan = apply_handoff(Some(plan), "research", "SECRET HAND-OFF", now()).unwrap();
        plan.phase_mut("build").unwrap().gate_state = Some(GateState {
            attempts: 2,
            last_passed: Some(false),
            last_error: Some("SECRET ERROR".into()),
            ..GateState::default()
        });
        let stored = plan.clone();
        mutate_plan(&manager, &parent, move |_| Ok((stored, ())))
            .await
            .unwrap();

        let (title, goal, phases) = declared_skeleton(&manager, &parent).await.unwrap();
        assert_eq!(title, "Ship the parser");
        assert_eq!(goal.as_deref(), Some("A parser with tests"));
        assert_eq!(phases.len(), 2);
        assert_eq!(phases[0].phase.id, "research");
        assert_eq!(phases[0].phase.goal.as_deref(), Some("Read the old code"));
        assert_eq!(phases[0].phase.kind.as_deref(), Some("research"));
        assert_eq!(phases[0].monitors, Some(vec![monitor_spec("Off task")]));
        assert!(phases[0].gate.is_none());
        assert_eq!(phases[1].phase.depends_on, vec!["research"]);
        assert_eq!(phases[1].monitors, None);
        let gate = phases[1].gate.clone().unwrap();
        assert_eq!(gate.threshold, Some(0.8));
        assert_eq!(gate.max_attempts, Some(4));
        assert_eq!(gate.model.as_deref(), Some("ollama_decision/tev1:0.8b"));
        assert_eq!(gate.questions.len(), 2);

        let json = serde_json::to_string(&phases).unwrap();
        for state in ["SECRET", "c1", "agents", "handoff", "status", "gate_state"] {
            assert!(!json.contains(state), "{state} leaked: {json}");
        }

        // It declares the same plan again, fresh, as a workflow.
        let again = apply_workflow_plan(&title, goal.as_deref(), phases, now()).unwrap();
        let build = again.phase("build").unwrap();
        assert!(same_gate_terms(
            build.gate.as_ref().unwrap(),
            plan.phase("build").unwrap().gate.as_ref().unwrap()
        ));
        assert_eq!(build.gate.as_ref().unwrap().owner, GateOwner::Workflow);
        assert!(build.gate_state.is_none());
        assert!(again
            .phases
            .iter()
            .all(|phase| phase.status == PhaseStatus::Pending
                && phase.agents.is_empty()
                && phase.handoff.is_none()));
        assert_eq!(
            phase_monitors_for(&again, "research"),
            vec![monitor_spec("Off task")]
        );
        assert_eq!(
            phase_monitors(&manager, &parent, "research").await.unwrap(),
            vec![monitor_spec("Off task")]
        );
    }

    /// The async wrappers write through the store in one transaction, keep
    /// each refusal's kind, and hold a pass while a spawn into the phase is in
    /// flight.
    #[tokio::test]
    async fn the_person_wrappers_write_through_the_store() {
        let (_temp, manager, parent) = store_with_parent().await;
        assert_eq!(
            person_remove_gate(&manager, &parent, "pw-build").await,
            Err(GateWriteError::NoPlan)
        );
        // Phase ids of their own: the spawn registry is process-global.
        let declared = apply_gated_plan(
            None,
            Some("Wrappers"),
            None,
            &[
                gated("pw-research", "research", &[], None),
                gated("pw-build", "implement", &[], Some(build_gate_input())),
                gated("pw-verify", "verify", &[], None),
            ],
        )
        .unwrap();
        mutate_plan(&manager, &parent, move |_| Ok((declared, ())))
            .await
            .unwrap();

        assert!(matches!(
            person_gate_decision(&manager, &parent, "deploy", PersonGateChoice::Pass).await,
            Err(GateWriteError::NoPhase(_))
        ));
        let outcome = person_set_gate(&manager, &parent, " pw-build ", person_gate("pw-build"))
            .await
            .unwrap();
        assert!(outcome.applied);
        let stored = load_plan(&manager, &parent).await.unwrap().unwrap();
        assert_eq!(stored.revision, 2);
        assert_eq!(
            stored
                .phase("pw-build")
                .unwrap()
                .gate
                .as_ref()
                .unwrap()
                .owner,
            GateOwner::Person
        );

        // A spawn into the phase in flight keeps the pass standing.
        let spawning = begin_phase_spawn(&parent, "pw-build");
        let outcome = person_gate_decision(&manager, &parent, "pw-build", PersonGateChoice::Pass)
            .await
            .unwrap();
        assert!(!outcome.applied);
        assert!(
            outcome
                .message
                .contains("once the subagent being started in it finishes"),
            "{}",
            outcome.message
        );
        drop(spawning);
        let stored = load_plan(&manager, &parent).await.unwrap().unwrap();
        assert_eq!(
            stored.phase("pw-build").unwrap().status,
            PhaseStatus::Pending
        );

        // Once nothing is in flight, the pass applies.
        let outcome = person_gate_decision(&manager, &parent, "pw-build", PersonGateChoice::Pass)
            .await
            .unwrap();
        assert!(outcome.applied, "{}", outcome.message);
        let stored = load_plan(&manager, &parent).await.unwrap().unwrap();
        assert_eq!(
            stored.phase("pw-build").unwrap().status,
            PhaseStatus::Completed
        );
        let revision = stored.revision;

        // A refusal writes nothing.
        assert!(matches!(
            person_remove_gate(&manager, &parent, "pw-build").await,
            Err(GateWriteError::Closed(_))
        ));
        assert!(matches!(
            person_remove_gate(&manager, &parent, "pw-verify").await,
            Err(GateWriteError::NoGate(_))
        ));
        assert_eq!(
            load_plan(&manager, &parent)
                .await
                .unwrap()
                .unwrap()
                .revision,
            revision
        );

        // A plan this build cannot read is never replaced.
        manager
            .update_extension_state(&parent, PLAN_STATE, PLAN_VERSION, |_| {
                Ok(serde_json::json!({ "title": 7 }))
            })
            .await
            .unwrap();
        assert!(matches!(
            person_set_gate(&manager, &parent, "pw-verify", person_gate("pw-verify")).await,
            Err(GateWriteError::Unreadable(_))
        ));
    }

    #[test]
    fn a_passing_gate_completes_the_phase() {
        let plan = gated_plan();
        let gate = stored_gate(&plan);
        let verdict =
            GateVerdict::Passed(answers(&[("tests_pass", 0.93), ("scope_met", 0.88)], 0.8));
        let (plan, note, closed) =
            apply_gate_verdict(Some(plan), "build", &gate, &verdict, None, &[], now()).unwrap();
        assert!(closed);
        let build = plan.phase("build").unwrap();
        assert_eq!(build.status, PhaseStatus::Completed);
        let state = build.gate_state.as_ref().unwrap();
        assert_eq!(state.attempts, 1);
        assert_eq!(state.last_passed, Some(true));
        assert_eq!(state.last_min_p, Some(0.88));
        assert!(note.contains("gate passed on attempt 1 of 3"), "{note}");
        assert!(note.contains("tests_pass P(yes) 0.93"), "{note}");
        let snap = snapshot_of(&plan, &[]).phases[1].gate.clone().unwrap();
        assert_eq!(snap.last_passed, Some(true));
        assert!(!snap.blocked);
    }

    /// A failure keeps the phase in progress and tells the model each failing
    /// question and its p; the last allowed failure blocks it.
    #[test]
    fn a_failing_gate_keeps_the_phase_open_until_its_attempts_are_used_then_blocks() {
        let mut plan = gated_plan();
        let gate = stored_gate(&plan);
        let fail = GateVerdict::Failed(answers(&[("tests_pass", 0.42), ("scope_met", 0.91)], 0.8));
        for attempt in 1..=2u32 {
            let (next, note, closed) = apply_gate_verdict(
                Some(plan),
                "build",
                &gate,
                &fail,
                Some("Tests: 3 of 4 pass."),
                &[],
                now(),
            )
            .unwrap();
            plan = next;
            assert!(!closed);
            let build = plan.phase("build").unwrap();
            assert_eq!(build.status, PhaseStatus::InProgress);
            assert_eq!(build.gate_state.as_ref().unwrap().attempts, attempt);
            assert!(note.contains("NOT completed"), "{note}");
            assert!(note.contains(&format!("attempt {attempt} of 3")), "{note}");
            assert!(note.contains("tests_pass: P(yes) 0.42"), "{note}");
            assert!(
                !note.contains("scope_met: P(yes)"),
                "a passing question is not listed: {note}"
            );
            assert_eq!(
                build.handoff.as_ref().unwrap().summary,
                "Tests: 3 of 4 pass.",
                "the summary given is kept as the hand-off for the next attempt"
            );
            assert!(refuse_exhausted_gate(build).is_none());
        }
        let (plan, note, _) =
            apply_gate_verdict(Some(plan), "build", &gate, &fail, None, &[], now()).unwrap();
        let build = plan.phase("build").unwrap();
        assert_eq!(build.status, PhaseStatus::Blocked);
        assert!(note.contains("is blocked"), "{note}");
        assert!(note.contains("Tell the person"), "{note}");
        let snap = snapshot_of(&plan, &[]).phases[1].gate.clone().unwrap();
        assert!(snap.blocked);
        assert_eq!((snap.attempts, snap.max_attempts), (3, 3));
        assert_eq!(snap.last_min_p, Some(0.42));
        let refusal = refuse_exhausted_gate(build).unwrap();
        assert!(refusal.contains("failed all 3 attempts"), "{refusal}");
    }

    /// An error never passes and uses no attempt; two in a row block.
    #[test]
    fn a_gate_error_uses_no_attempt_and_two_in_a_row_block() {
        let plan = gated_plan();
        let gate = stored_gate(&plan);
        let error = GateVerdict::Error("the decision server failed".into());
        let (plan, note, closed) =
            apply_gate_verdict(Some(plan), "build", &gate, &error, None, &[], now()).unwrap();
        assert!(!closed);
        let build = plan.phase("build").unwrap();
        assert_eq!(
            build.status,
            PhaseStatus::Pending,
            "an error changes no status"
        );
        let state = build.gate_state.clone().unwrap();
        assert_eq!((state.attempts, state.consecutive_errors), (0, 1));
        assert!(note.contains("no attempt was used"), "{note}");
        assert!(note.contains("the decision server failed"), "{note}");

        let (plan, note, _) =
            apply_gate_verdict(Some(plan), "build", &gate, &error, None, &[], now()).unwrap();
        let build = plan.phase("build").unwrap();
        assert_eq!(build.status, PhaseStatus::Blocked);
        assert!(note.contains("2 times in a row"), "{note}");
        assert!(
            snapshot_of(&plan, &[]).phases[1]
                .gate
                .as_ref()
                .unwrap()
                .blocked
        );
        assert!(
            refuse_exhausted_gate(build).is_none(),
            "an error block is not an exhausted gate: completing again retries"
        );

        // Once the model is reachable again, a pass completes and clears the errors.
        let pass = GateVerdict::Passed(answers(&[("tests_pass", 0.9), ("scope_met", 0.9)], 0.8));
        let (plan, _, closed) =
            apply_gate_verdict(Some(plan), "build", &gate, &pass, None, &[], now()).unwrap();
        assert!(closed);
        let state = plan.phase("build").unwrap().gate_state.clone().unwrap();
        assert_eq!((state.attempts, state.consecutive_errors), (1, 0));
    }

    #[test]
    fn a_verdict_is_refused_when_the_gate_changed_or_the_barrier_no_longer_holds() {
        let plan = gated_plan();
        let gate = stored_gate(&plan);
        let other = PhaseGate {
            threshold: 0.5,
            ..gate.clone()
        };
        let pass = GateVerdict::Passed(answers(&[("tests_pass", 0.9), ("scope_met", 0.9)], 0.8));
        let err = apply_gate_verdict(Some(plan.clone()), "build", &other, &pass, None, &[], now())
            .unwrap_err();
        assert!(
            err.contains("changed while it was being evaluated"),
            "{err}"
        );

        let busy = with_agents(plan, "build", &["c1"]);
        let err = apply_gate_verdict(
            Some(busy),
            "build",
            &gate,
            &pass,
            None,
            &[state("c1", SubagentRunStatus::Running, None)],
            now(),
        )
        .unwrap_err();
        assert!(err.contains("cannot be completed yet"), "{err}");
    }

    /// The gate's model reads the goal, the hand-off and the children's result
    /// lines, and never a withheld child's text.
    #[test]
    fn the_gate_state_carries_results_but_never_a_withheld_childs_text() {
        let mut plan = with_agents(gated_plan(), "build", &["c1", "c2"]);
        plan.phase_mut("build").unwrap().goal = Some("A parser with tests".into());
        plan = apply_handoff(Some(plan), "build", "Parser done; 12 tests added.", now()).unwrap();
        let mut withheld = state("c2", SubagentRunStatus::Completed, None);
        withheld.withheld = true;
        let agents = [
            state(
                "c1",
                SubagentRunStatus::Completed,
                Some("All 12 tests pass"),
            ),
            withheld,
        ];
        let text = gate_state_text(plan.phase("build").unwrap(), &agents);
        assert!(text.contains("Goal: A parser with tests"), "{text}");
        assert!(text.contains("Parser done; 12 tests added."), "{text}");
        assert!(
            text.contains("c1 role [completed]: All 12 tests pass"),
            "{text}"
        );
        assert!(
            text.contains("c2 role [completed]: (result withheld)"),
            "{text}"
        );

        // An automatic hand-off already holds the lines; they are not repeated.
        let (auto, _) = apply_complete(
            Some(with_agents(gated_plan(), "build", &["c1"])),
            "build",
            PhaseStatus::Completed,
            None,
            &agents[..1],
            now(),
        )
        .unwrap();
        let text = gate_state_text(auto.phase("build").unwrap(), &agents[..1]);
        assert_eq!(text.matches("All 12 tests pass").count(), 1, "{text}");

        let longest = "x".repeat(MAX_HANDOFF_CHARS);
        let plan = apply_handoff(Some(gated_plan()), "build", &longest, now()).unwrap();
        let line = "y".repeat(200);
        let many: Vec<PhaseAgentState> = (0..40)
            .map(|i| state(&format!("c{i}"), SubagentRunStatus::Completed, Some(&line)))
            .collect();
        assert!(
            gate_state_text(plan.phase("build").unwrap(), &many)
                .chars()
                .count()
                <= GATE_STATE_MAX_CHARS
        );
    }

    /// F1 (live QA 2026-11-10): a gate's state is fitted to the gate model's
    /// window, as a monitor's window is. The QA's shape, a phase whose 40
    /// children reported lines full of numbers cut at 6,000 characters, was
    /// refused by `tev1:0.8b`'s 2,048-token window ("about 3878 tokens");
    /// fitted, the size check's own estimate admits it, and the phase and its
    /// goal survive at the start.
    #[tokio::test]
    async fn a_gate_state_is_fitted_to_the_gate_models_window() {
        use crate::decision::service::{context_margin, estimate_tokens, load_token_counter};
        load_token_counter().await;
        let mut plan = gated_plan();
        plan.phase_mut("build").unwrap().goal = Some("Analyse every sample".into());
        let line = "Ran 14 samples on 2026-11-05, mean glucose 104.2 mg/dL (SD 3.1), HbA1c \
                    6.1%, p=0.0042, n=1204, output /data/run_20261105_0913/result.csv";
        let agents: Vec<PhaseAgentState> = (0..40)
            .map(|i| {
                state(
                    &format!("worker-{i}"),
                    SubagentRunStatus::Completed,
                    Some(line),
                )
            })
            .collect();
        let phase = plan.phase("build").unwrap();
        let gate = phase.gate.as_ref().unwrap();
        let questions: IndexMap<String, Question> = gate
            .questions
            .iter()
            .map(|(name, question)| {
                (
                    name.clone(),
                    Question {
                        threshold: Some(Threshold::Yes(gate.threshold)),
                        ..question.clone()
                    },
                )
            })
            .collect();
        let state_text = gate_state_text(phase, &agents);
        let fits = |text: &str| {
            let request = DecisionRequest {
                questions: questions.clone(),
                context: DecisionContext::Text(text.to_string()),
            };
            async move { estimate_tokens(&request).await + context_margin(2_048) <= 2_048 }
        };
        assert!(
            !fits(&state_text).await,
            "the fixture must overflow the window"
        );

        let fitted = fit_state_to_window(&state_text, 2_048, &questions);
        assert!(
            fits(&fitted).await,
            "{} chars still overflow",
            fitted.chars().count()
        );
        assert!(fitted.starts_with("Phase \""), "{fitted}");
        assert!(fitted.contains("Goal: Analyse every sample"), "{fitted}");
        assert!(fitted.chars().count() >= GATE_STATE_MIN_CHARS);

        // A model with room for it reads the state unchanged.
        assert_eq!(
            fit_state_to_window(&state_text, 16_000, &questions),
            state_text
        );
    }

    #[test]
    fn the_plan_text_and_the_moim_show_a_gate() {
        let plan = gated_plan();
        let snap = snapshot_of(&plan, &[]);
        let gate = snap.phases[1].gate.clone().unwrap();
        assert_eq!(gate.question_count, 2);
        assert_eq!(gate.attempts, 0);
        assert!(snap.phases[0].gate.is_none());
        let text = render_snapshot_text(&snap);
        assert!(
            text.contains("gate: 2 questions at P(yes) >= 0.80, 0 of 3 attempts used"),
            "{text}"
        );
        assert!(render_moim(&snap).contains("gated (attempt 0 of 3)"));
        let json = serde_json::to_value(&snap).unwrap();
        assert_eq!(json["phases"][1]["gate"]["maxAttempts"], 3);
        assert!(json["phases"][0]["gate"].is_null());
    }

    /// A plan stored before gates existed still reads.
    #[test]
    fn a_plan_stored_without_gates_still_parses() {
        let mut value = serde_json::to_value(three_phase_plan()).unwrap();
        for phase in value["phases"].as_array_mut().unwrap() {
            let object = phase.as_object_mut().unwrap();
            object.remove("gate");
            object.remove("gate_state");
        }
        let plan = parse_stored(&value).unwrap();
        assert!(plan.phases.iter().all(|p| p.gate.is_none()));
    }

    mod gate_evaluation {
        use super::*;
        use crate::agents::decisions_extension::test_support::{entry, model, service};
        use crate::decision::DecisionAdapter;
        use crate::privacy::ProviderTier;
        use std::sync::atomic::Ordering;

        fn session(private: bool, enforced: bool) -> DecisionSession {
            DecisionSession::new(
                "gate-session",
                if private {
                    SessionClassification::Private
                } else {
                    SessionClassification::Public
                },
                enforced,
            )
            .with_origin(crate::decision::DecisionOrigin::Gate, Some("build".into()))
        }

        fn gate_with(questions: &[(&str, &str)], allow_uncalibrated: bool) -> PhaseGate {
            validate_gate(
                "build",
                &PhaseGateInput {
                    allow_uncalibrated: Some(allow_uncalibrated),
                    ..gate_input(questions)
                },
            )
            .unwrap()
        }

        #[tokio::test]
        async fn every_question_at_the_threshold_passes_and_one_below_fails() {
            let local = model(
                entry(
                    "tev1",
                    ProviderTier::Private,
                    true,
                    DecisionAdapter::SystemOne,
                ),
                0.95,
            );
            let svc = service(vec![local]);
            let gate = gate_with(&[("a", "Is a done?"), ("b", "Is b done? [p=0.8]")], false);
            let verdict = evaluate_gate(svc, &session(true, true), &gate, "state".into()).await;
            let GateVerdict::Passed(answers) = verdict else {
                panic!("{verdict:?}")
            };
            assert_eq!(answers.len(), 2);
            assert!(
                (answers[1].p_yes - 0.8).abs() < 1e-9,
                "equal to the threshold passes"
            );

            let gate = gate_with(&[("a", "Is a done?"), ("b", "Is b done? [p=0.42]")], false);
            let verdict = evaluate_gate(svc, &session(true, true), &gate, "state".into()).await;
            let GateVerdict::Failed(answers) = verdict else {
                panic!("{verdict:?}")
            };
            assert!(answers[0].passed && !answers[1].passed);
            assert!((answers[1].p_yes - 0.42).abs() < 1e-9);
        }

        /// A gate that names no model asks the chat's own default decision
        /// model, read with the chat as the gate path reads it, ahead of the
        /// configured default.
        #[tokio::test]
        async fn a_gate_without_a_model_asks_the_chats_default_decision_model() {
            use crate::decision::settings::{save, DecisionSettings};
            use crate::decision::DecisionModelRef;
            use crate::session::{SessionManager, SessionType};

            let local = model(
                entry(
                    "tev1",
                    ProviderTier::Private,
                    true,
                    DecisionAdapter::SystemOne,
                ),
                0.95,
            );
            let cloud = model(
                entry(
                    "cloud",
                    ProviderTier::Public,
                    false,
                    DecisionAdapter::SystemOne,
                ),
                0.95,
            );
            let (local_calls, cloud_calls) = (local.calls.clone(), cloud.calls.clone());
            let svc = crate::agents::decisions_extension::test_support::service_with_default(
                vec![local, cloud],
                Some(DecisionModelRef::new("test_decision", "cloud")),
            );
            let dir = tempfile::tempdir().unwrap();
            let manager = Arc::new(SessionManager::new_ephemeral(dir.path().join("data")));
            let chat = manager
                .create_session(dir.path().to_path_buf(), "gated".into(), SessionType::User)
                .await
                .unwrap();
            save(
                &manager,
                &chat.id,
                DecisionSettings {
                    default_model: Some(DecisionModelRef::new("test_decision", "tev1")),
                    monitors: Vec::new(),
                },
            )
            .await
            .unwrap();
            let session = DecisionSession::load(manager, &chat.id, true)
                .await
                .unwrap()
                .with_origin(crate::decision::DecisionOrigin::Gate, Some("build".into()));
            let gate = gate_with(&[("a", "Is a done?")], false);
            assert!(gate.model.is_none());

            let verdict = evaluate_gate(svc, &session, &gate, "state".into()).await;

            assert!(matches!(verdict, GateVerdict::Passed(_)), "{verdict:?}");
            assert_eq!(local_calls.load(Ordering::SeqCst), 1, "the chat's default");
            assert_eq!(
                cloud_calls.load(Ordering::SeqCst),
                0,
                "not the configured one"
            );
        }

        #[tokio::test]
        async fn an_error_never_passes() {
            let mut broken = model(
                entry(
                    "tev1",
                    ProviderTier::Private,
                    true,
                    DecisionAdapter::SystemOne,
                ),
                0.99,
            );
            broken.fail_all = true;
            let svc = service(vec![broken]);
            let gate = gate_with(&[("a", "Is a done?")], false);
            let verdict = evaluate_gate(svc, &session(false, true), &gate, "state".into()).await;
            assert!(matches!(verdict, GateVerdict::Error(_)), "{verdict:?}");
        }

        #[tokio::test]
        async fn a_verbalized_source_is_refused_unless_the_gate_allows_it() {
            let verbal = model(
                entry(
                    "chatty",
                    ProviderTier::Private,
                    true,
                    DecisionAdapter::Verbalized,
                ),
                0.99,
            );
            let svc = service(vec![verbal]);
            let strict = gate_with(&[("a", "Is a done?")], false);
            let verdict = evaluate_gate(svc, &session(false, true), &strict, "s".into()).await;
            let GateVerdict::Error(message) = verdict else {
                panic!("{verdict:?}")
            };
            assert!(message.contains("uncalibrated"), "{message}");
            let lenient = gate_with(&[("a", "Is a done?")], true);
            let verdict = evaluate_gate(svc, &session(false, true), &lenient, "s".into()).await;
            assert!(matches!(verdict, GateVerdict::Passed(_)), "{verdict:?}");
        }

        /// Gate H: a private parent's phase results never reach a public
        /// decision model, and nothing is sent.
        #[tokio::test]
        async fn a_private_chats_gate_never_reaches_a_public_model() {
            let cloud = model(
                entry(
                    "cloud",
                    ProviderTier::Public,
                    false,
                    DecisionAdapter::SystemOne,
                ),
                0.99,
            );
            let calls = cloud.calls.clone();
            let svc = service(vec![cloud]);
            let gate = gate_with(&[("a", "Is a done?")], false);
            let verdict = evaluate_gate(svc, &session(true, true), &gate, "PHI".into()).await;
            assert!(matches!(verdict, GateVerdict::Error(_)), "{verdict:?}");
            let named = PhaseGate {
                model: Some(DecisionModelRef::new("test_decision", "cloud")),
                ..gate
            };
            let verdict = evaluate_gate(svc, &session(true, true), &named, "PHI".into()).await;
            let GateVerdict::Error(message) = verdict else {
                panic!("{verdict:?}")
            };
            assert!(message.contains("private"), "{message}");
            assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing may be sent");
        }

        // -------------------------------------------------------------------
        // The chat's own model judges a gate that names no model when no
        // decision model may.
        // -------------------------------------------------------------------

        use super::super::gate_chat_stub::ChatStub;

        fn answer(p: f64) -> String {
            let word = if p >= 0.5 { "yes" } else { "no" };
            serde_json::json!({ "a": { "answer": word, "p_yes": p } }).to_string()
        }

        /// With no decision model at all, the chat's own model reads the same
        /// gate text and its verdict passes or holds the gate by the threshold.
        /// The gate's record says who judged it and why.
        #[tokio::test]
        async fn with_no_decision_model_the_chats_own_model_judges_the_gate() {
            let svc = service(Vec::new());
            let gate = gate_with(&[("a", "Is a done?")], false);
            let chat = ChatStub::answering(&answer(0.92));
            let (verdict, judge) = judge_gate(
                svc,
                &session(false, true),
                &gate,
                "Hand-off: a is done".into(),
                Some(chat.provider()),
            )
            .await;
            let GateVerdict::Passed(answers) = verdict else {
                panic!("{verdict:?}")
            };
            assert!((answers[0].p_yes - 0.92).abs() < 1e-9);
            assert_eq!(chat.calls(), 1, "one short completion");
            let prompt = chat.last_prompt();
            assert!(prompt.contains("Hand-off: a is done"), "{prompt}");
            assert!(prompt.contains("- a: Is a done?"), "{prompt}");
            let judge = judge.expect("a verdict names who gave it");
            assert_eq!(judge.kind, GateJudgeKind::ChatModel);
            assert_eq!(judge.model, "stub-chat-model");
            assert_eq!(judge.words(), "stub-chat-model, the chat's model");
            assert_eq!(
                judge.reason.as_deref(),
                Some(
                    "This chat has no decision model it may use, so stub-chat-model, the \
                     chat's model, judged the gate."
                )
            );

            // Below the threshold (0.8): the gate holds, as a decision
            // model's verdict would.
            let chat = ChatStub::answering(&answer(0.3));
            let (verdict, judge) = judge_gate(
                svc,
                &session(false, true),
                &gate,
                "state".into(),
                Some(chat.provider()),
            )
            .await;
            let GateVerdict::Failed(answers) = verdict else {
                panic!("{verdict:?}")
            };
            assert!(!answers[0].passed);
            assert!(judge.is_some());

            // Exactly at the threshold passes (the noul rule).
            let chat = ChatStub::answering(&answer(0.8));
            let (verdict, _) = judge_gate(
                svc,
                &session(false, true),
                &gate,
                "state".into(),
                Some(chat.provider()),
            )
            .await;
            assert!(matches!(verdict, GateVerdict::Passed(_)), "{verdict:?}");
        }

        /// A private chat whose only decision model is public: the decision
        /// model is never asked, and the chat's own (private) model judges.
        #[tokio::test]
        async fn a_private_chat_with_only_a_public_decision_model_uses_its_own_model() {
            let cloud = model(
                entry(
                    "cloud",
                    ProviderTier::Public,
                    false,
                    DecisionAdapter::SystemOne,
                ),
                0.99,
            );
            let calls = cloud.calls.clone();
            let svc = service(vec![cloud]);
            let gate = gate_with(&[("a", "Is a done?")], false);
            let chat = ChatStub::answering(&answer(0.9));
            let (verdict, judge) = judge_gate(
                svc,
                &session(true, true),
                &gate,
                "PHI".into(),
                Some(chat.provider()),
            )
            .await;
            assert!(matches!(verdict, GateVerdict::Passed(_)), "{verdict:?}");
            assert_eq!(judge.unwrap().kind, GateJudgeKind::ChatModel);
            assert_eq!(
                calls.load(Ordering::SeqCst),
                0,
                "nothing reaches the public model"
            );
            assert_eq!(chat.calls(), 1);
        }

        /// A usable decision model still judges a gate that names none, and
        /// the chat's model is not asked.
        #[tokio::test]
        async fn a_usable_decision_model_judges_and_the_chat_model_is_not_asked() {
            let local = model(
                entry(
                    "tev1",
                    ProviderTier::Private,
                    true,
                    DecisionAdapter::SystemOne,
                ),
                0.95,
            );
            let svc = service(vec![local]);
            let gate = gate_with(&[("a", "Is a done?")], false);
            let chat = ChatStub::answering(&answer(0.1));
            let (verdict, judge) = judge_gate(
                svc,
                &session(true, true),
                &gate,
                "state".into(),
                Some(chat.provider()),
            )
            .await;
            assert!(matches!(verdict, GateVerdict::Passed(_)), "{verdict:?}");
            let judge = judge.unwrap();
            assert_eq!(judge.kind, GateJudgeKind::DecisionModel);
            assert_eq!(judge.model, "tev1");
            assert_eq!(judge.reason, None);
            assert_eq!(chat.calls(), 0);
        }

        /// A decision model that cannot be reached (its server failed, here)
        /// hands a gate that names no model to the chat's own model, and the
        /// record says why in one plain sentence. A gate that NAMES the model
        /// keeps the error and never asks the chat's model.
        #[tokio::test]
        async fn an_unreachable_decision_model_falls_back_only_for_a_gate_that_names_none() {
            let mut broken = model(
                entry(
                    "joey-1",
                    ProviderTier::Private,
                    true,
                    DecisionAdapter::SystemOne,
                ),
                0.99,
            );
            broken.fail_all = true;
            let calls = broken.calls.clone();
            let svc = service(vec![broken]);
            let gate = gate_with(&[("a", "Is a done?")], false);
            let chat = ChatStub::answering(&answer(0.9));
            let (verdict, judge) = judge_gate(
                svc,
                &session(false, true),
                &gate,
                "state".into(),
                Some(chat.provider()),
            )
            .await;
            assert!(matches!(verdict, GateVerdict::Passed(_)), "{verdict:?}");
            assert_eq!(
                calls.load(Ordering::SeqCst),
                1,
                "the decision model was tried"
            );
            assert_eq!(chat.calls(), 1);
            let judge = judge.unwrap();
            assert_eq!(judge.kind, GateJudgeKind::ChatModel);
            assert_eq!(
                judge.reason.as_deref(),
                Some(
                    "joey-1 failed on its server (HTTP 500), so stub-chat-model, the chat's \
                     model, judged the gate."
                )
            );

            // A transport failure reads as the adapter's own plain sentence.
            let unreachable = crate::decision::DecisionError::Backend {
                model: "decision-models/joey".into(),
                status: None,
                error_type: None,
                message: "joey-1 on Baseten could not be reached because its server's \
                          certificate did not match"
                    .into(),
            };
            assert_eq!(
                unreachable_clause(&unreachable, "joey-1 on Baseten").as_deref(),
                Some(
                    "joey-1 on Baseten could not be reached because its server's certificate \
                     did not match"
                )
            );
            // A refusal, a malformed answer and a rate limit are not
            // "unreachable": they stay errors.
            for kept in [
                crate::decision::DecisionError::Refused("no".into()),
                crate::decision::DecisionError::malformed("joey-1", "no answer"),
                crate::decision::DecisionError::Backend {
                    model: "m".into(),
                    status: Some(429),
                    error_type: None,
                    message: "rate limited".into(),
                },
            ] {
                assert_eq!(unreachable_clause(&kept, "joey-1"), None, "{kept:?}");
            }

            let named = PhaseGate {
                model: Some(DecisionModelRef::new("test_decision", "joey-1")),
                ..gate
            };
            let chat = ChatStub::answering(&answer(0.9));
            let (verdict, judge) = judge_gate(
                svc,
                &session(false, true),
                &named,
                "state".into(),
                Some(chat.provider()),
            )
            .await;
            assert!(matches!(verdict, GateVerdict::Error(_)), "{verdict:?}");
            assert!(judge.is_none());
            assert_eq!(chat.calls(), 0, "a named model's gate never falls back");
        }

        /// The chat model's answer is read strictly: anything but one JSON
        /// object answering every question, yes or no with a P(yes) that agrees
        /// with it, holds the gate as an error (never a pass, no attempt used).
        #[tokio::test]
        async fn a_malformed_or_failed_chat_model_answer_never_passes() {
            let svc = service(Vec::new());
            let gate = gate_with(&[("a", "Is a done?")], false);
            let cases = [
                ("Yes, a is done.", "not one JSON object"),
                (r#"{"b": {"answer": "yes", "p_yes": 0.9}}"#, "does not ask"),
                (r#"{}"#, "did not answer a"),
                (r#"{"a": {"answer": "yes", "p_yes": 0.2}}"#, "disagreed"),
                (
                    r#"{"a": {"answer": "maybe", "p_yes": 0.9}}"#,
                    "not yes or no",
                ),
                (
                    r#"{"a": {"answer": "yes", "p_yes": 1.5}}"#,
                    "between 0 and 1",
                ),
                (
                    r#"{"a": {"answer": "yes", "p_yes": "0.9"}}"#,
                    "not one JSON object",
                ),
                (
                    r#"{"a": {"answer": "yes", "p_yes": 0.9, "why": "x"}}"#,
                    "not one JSON object",
                ),
            ];
            for (reply, needle) in cases {
                let chat = ChatStub::answering(reply);
                let (verdict, judge) = judge_gate(
                    svc,
                    &session(false, true),
                    &gate,
                    "state".into(),
                    Some(chat.provider()),
                )
                .await;
                let GateVerdict::Error(message) = verdict else {
                    panic!("{reply}: {verdict:?}")
                };
                assert!(message.contains(needle), "{reply}: {message}");
                assert!(
                    message.contains("stub-chat-model, the chat's model"),
                    "{message}"
                );
                assert!(judge.is_none(), "an error names no judge");
            }

            // A code fence around the object is the one leniency.
            let chat = ChatStub::answering(&format!("```json\n{}\n```", answer(0.9)));
            let (verdict, _) = judge_gate(
                svc,
                &session(false, true),
                &gate,
                "state".into(),
                Some(chat.provider()),
            )
            .await;
            assert!(matches!(verdict, GateVerdict::Passed(_)), "{verdict:?}");

            // The chat model fails to answer: an error.
            let chat = ChatStub::failing();
            let (verdict, judge) = judge_gate(
                svc,
                &session(false, true),
                &gate,
                "state".into(),
                Some(chat.provider()),
            )
            .await;
            let GateVerdict::Error(message) = verdict else {
                panic!("{verdict:?}")
            };
            assert!(
                message.contains("could not judge the gate either"),
                "{message}"
            );
            assert!(judge.is_none());

            // No chat model to ask: the decision side's error stands.
            let (verdict, judge) =
                judge_gate(svc, &session(false, true), &gate, "state".into(), None).await;
            let GateVerdict::Error(message) = verdict else {
                panic!("{verdict:?}")
            };
            assert!(message.contains("was not available"), "{message}");
            assert!(judge.is_none());

            // Written through the plan, an error uses no attempt and the
            // gate stays open.
            let plan = gated_plan();
            let stored = stored_gate(&plan);
            let (plan, note, closed) = apply_gate_verdict(
                Some(plan),
                "build",
                &stored,
                &GateVerdict::Error(message),
                None,
                &[],
                now(),
            )
            .unwrap();
            assert!(!closed);
            assert!(note.contains("no attempt was used"), "{note}");
            let gate = snapshot_of(&plan, &[]).phases[1].gate.clone().unwrap();
            assert_eq!(gate.attempts, 0);
            assert_eq!(gate.last_passed, None);
        }

        /// A verdict's judge is kept on the gate and shown in its snapshot.
        #[test]
        fn the_judge_of_a_verdict_is_kept_with_it() {
            let plan = gated_plan();
            let stored = stored_gate(&plan);
            let pass =
                GateVerdict::Passed(answers(&[("tests_pass", 0.9), ("scope_met", 0.9)], 0.8));
            let (mut plan, _, closed) =
                apply_gate_verdict(Some(plan), "build", &stored, &pass, None, &[], now()).unwrap();
            assert!(closed);
            let judge = GateJudge {
                kind: GateJudgeKind::ChatModel,
                model: "gpt-5.5".into(),
                reason: Some(
                    "This chat has no decision model it may use, so gpt-5.5, the \
                              chat's model, judged the gate."
                        .into(),
                ),
            };
            record_gate_judge(&mut plan, "build", &judge);
            let gate = snapshot_of(&plan, &[]).phases[1].gate.clone().unwrap();
            assert_eq!(gate.last_judge, Some(judge));
            assert_eq!(gate.last_passed, Some(true));
        }
    }
}

/// A chat model for gate tests: answers every completion with fixed text (or
/// fails), and keeps the last prompt it read.
#[cfg(test)]
pub(crate) mod gate_chat_stub {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use rmcp::model::Tool;

    use crate::conversation::message::Message;
    use crate::model::ModelConfig;
    use crate::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
    use crate::providers::errors::ProviderError;

    pub(crate) struct ChatStub {
        reply: Option<String>,
        usage: Usage,
        calls: Arc<AtomicUsize>,
        last_prompt: Arc<Mutex<String>>,
    }

    #[derive(Clone)]
    pub(crate) struct ChatStubHandle {
        calls: Arc<AtomicUsize>,
        last_prompt: Arc<Mutex<String>>,
        provider: Arc<dyn Provider>,
    }

    impl ChatStub {
        pub(crate) fn answering(reply: &str) -> ChatStubHandle {
            Self::build(Some(reply.to_string()), Usage::default())
        }

        /// [`Self::answering`], reporting `usage` for each answer as a billed
        /// provider would.
        pub(crate) fn answering_with_usage(reply: &str, usage: Usage) -> ChatStubHandle {
            Self::build(Some(reply.to_string()), usage)
        }

        pub(crate) fn failing() -> ChatStubHandle {
            Self::build(None, Usage::default())
        }

        fn build(reply: Option<String>, usage: Usage) -> ChatStubHandle {
            let calls = Arc::new(AtomicUsize::new(0));
            let last_prompt = Arc::new(Mutex::new(String::new()));
            let provider = Arc::new(ChatStub {
                reply,
                usage,
                calls: calls.clone(),
                last_prompt: last_prompt.clone(),
            });
            ChatStubHandle {
                calls,
                last_prompt,
                provider,
            }
        }
    }

    impl ChatStubHandle {
        pub(crate) fn provider(&self) -> Arc<dyn Provider> {
            self.provider.clone()
        }

        pub(crate) fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        pub(crate) fn last_prompt(&self) -> String {
            self.last_prompt.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Provider for ChatStub {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::new(
                "stub-chat",
                "Stub chat",
                "",
                "stub-chat-model",
                vec!["stub-chat-model"],
                "",
                vec![],
            )
        }

        fn get_name(&self) -> &str {
            "stub-chat"
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last_prompt.lock().unwrap() = messages
                .iter()
                .map(Message::as_concat_text)
                .collect::<Vec<_>>()
                .join("\n");
            match &self.reply {
                Some(text) => Ok((
                    Message::assistant().with_text(text),
                    ProviderUsage::new("stub-chat-model".to_string(), self.usage),
                )),
                None => Err(ProviderError::ExecutionError("the stub failed".to_string())),
            }
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail("stub-chat-model")
        }
    }
}
