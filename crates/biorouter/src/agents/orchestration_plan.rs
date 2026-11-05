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
//! in a row block the phase. Each evaluation is one row of the decision log
//! (origin `gate`, origin id the phase id), never a transcript row.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::agents::subagent_roster::{SubagentRow, SubagentRunStatus};
use crate::decision::types::validate_questions;
use crate::decision::{
    DecisionContext, DecisionKind, DecisionModelRef, DecisionRequest, DecisionService,
    DecisionSession, Question, Threshold,
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
/// Evaluations in a row that end in an error before the phase is blocked.
pub const GATE_ERRORS_TO_BLOCK: u32 = 2;
/// The state a gate's decision model reads, at most. Sized like the monitor
/// window's default, so it fits a small local decision model.
pub const GATE_STATE_MAX_CHARS: usize = 6_000;

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

/// A decision-model gate on a phase (decision models, wave 2): before the phase
/// can complete, a decision model reads its hand-off and its children's result
/// lines and every question must reach P(yes) >= `threshold`. A failed gate keeps
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
}

fn default_gate_attempts() -> u32 {
    3
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
    pub blocked: bool,
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
// The stored model. Private to the crate: every reader outside it goes through
// `snapshot`, which joins it with the roster.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct OrchestrationPlan {
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
pub(crate) struct PlanPhase {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_at: Option<DateTime<Utc>>,
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

    /// Phases a child may still be spawned into.
    fn open_phase_ids(&self) -> String {
        join_ids(
            self.phases
                .iter()
                .filter(|phase| !phase.status.is_closed())
                .map(|phase| phase.id.as_str()),
        )
    }

    fn unknown_phase(&self, id: &str) -> String {
        format!(
            "Phase \"{id}\" is not in this chat's plan. Its phases are: {}.",
            self.phase_ids()
        )
    }
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
pub(crate) struct PlanPhaseInput {
    /// Short slug, unique in the plan: lowercase letters, digits and hyphens,
    /// at most 40 characters (for example "research" or "verify-api").
    pub(crate) id: String,
    /// What the phase is, in a few words (at most 80 characters).
    pub(crate) title: String,
    /// plan, research, execution, implementation, verification, testing,
    /// auditing, review, meeting, synthesis or other. Any other word is
    /// recorded as other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) kind: Option<String>,
    /// What the phase must achieve (at most 500 characters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) goal: Option<String>,
    /// Ids of EARLIER phases in this list whose hand-offs this phase's children
    /// receive. Empty means: every earlier phase that has completed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) depends_on: Vec<String>,
}

/// One phase as `workspace_orchestrate action:"plan"` takes it: the phase,
/// and optionally a decision gate on its completion.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub(crate) struct OrchestratePhaseInput {
    #[serde(flatten)]
    pub(crate) phase: PlanPhaseInput,
    /// Optional decision gate: yes/no questions a decision model answers from
    /// the phase's hand-off and its subagents' results before the phase may
    /// complete. Every question must reach the threshold. Leave it out of a
    /// revision to keep the phase's gate as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) gate: Option<PhaseGateInput>,
}

/// A phase gate as the model declares it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub(crate) struct PhaseGateInput {
    /// 1 to 8 yes/no questions, name to question, each answerable from the
    /// phase's results, for example {"tests_pass": "Do the results show every
    /// new test passing?"}. Names are letters, digits, `_` or `-`.
    #[schemars(with = "std::collections::BTreeMap<String, String>")]
    pub(crate) questions: IndexMap<String, String>,
    /// The P(yes) every question must reach, from 0 to 1. Default 0.8.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) threshold: Option<f64>,
    /// Failed evaluations allowed before the phase is blocked, 1 to 5.
    /// Default 3.
    #[serde(
        default,
        alias = "maxAttempts",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) max_attempts: Option<u32>,
    /// Accept verbalized (uncalibrated) probabilities. Default false: such a
    /// source is refused.
    #[serde(
        default,
        alias = "allowUncalibrated",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) allow_uncalibrated: Option<bool>,
    /// Optional decision model as provider/model, from decision_models. Left
    /// out, the configured default or the first model this chat may use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) model: Option<String>,
}

/// Validate a declared gate into the stored [`PhaseGate`].
pub(crate) fn validate_gate(phase_id: &str, input: &PhaseGateInput) -> Result<PhaseGate, String> {
    let count = input.questions.len();
    if count == 0 || count > MAX_GATE_QUESTIONS {
        return Err(format!(
            "Phase \"{phase_id}\"'s gate needs 1 to {MAX_GATE_QUESTIONS} yes/no questions, not \
             {count}."
        ));
    }
    let questions: IndexMap<String, Question> = input
        .questions
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
    let threshold = input.threshold.unwrap_or(DEFAULT_GATE_THRESHOLD);
    if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
        return Err(format!(
            "Phase \"{phase_id}\"'s gate threshold must be a probability from 0 to 1."
        ));
    }
    let max_attempts = input.max_attempts.unwrap_or_else(default_gate_attempts);
    if !(1..=MAX_GATE_ATTEMPTS).contains(&max_attempts) {
        return Err(format!(
            "Phase \"{phase_id}\"'s gate may allow 1 to {MAX_GATE_ATTEMPTS} attempts, not \
             {max_attempts}."
        ));
    }
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
    Ok(PhaseGate {
        questions,
        threshold,
        model,
        max_attempts,
        allow_uncalibrated: input.allow_uncalibrated.unwrap_or(false),
    })
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

/// One declared phase, validated against the phases listed before it
/// (`earlier`) and merged with its stored self when its id persists.
///
/// A gate given replaces the stored one; when it differs, the gate's record
/// starts over. A gate left out keeps the stored gate and its record, so a
/// revision that lists the phases again without their gates loses none.
fn build_phase(
    input: &PlanPhaseInput,
    gate: Option<&PhaseGateInput>,
    earlier: &[&str],
    existing: Option<&OrchestrationPlan>,
) -> Result<PlanPhase, String> {
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
    let mut depends_on: Vec<String> = Vec::new();
    for dependency in &input.depends_on {
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
    let kind = input
        .kind
        .as_deref()
        .map(PhaseKind::parse_lenient)
        .unwrap_or(PhaseKind::Other);
    let gate = gate.map(|gate| validate_gate(id, gate)).transpose()?;

    Ok(match existing.and_then(|plan| plan.phase(id)).cloned() {
        Some(previous) => {
            let (gate, gate_state) = match gate {
                None => (previous.gate.clone(), previous.gate_state.clone()),
                Some(gate) if previous.gate.as_ref() == Some(&gate) => {
                    (Some(gate), previous.gate_state.clone())
                }
                Some(gate) => (Some(gate), None),
            };
            PlanPhase {
                id: id.to_string(),
                title,
                goal,
                kind,
                depends_on,
                gate,
                gate_state,
                ..previous
            }
        }
        None => PlanPhase {
            id: id.to_string(),
            title,
            goal,
            kind,
            status: PhaseStatus::Pending,
            depends_on,
            agents: Vec::new(),
            handoff: None,
            started_at: None,
            finished_at: None,
            gate,
            gate_state: None,
        },
    })
}

/// [`apply_gated_plan`] with no gates declared, the shape the tests write.
#[cfg(test)]
pub(crate) fn apply_plan(
    existing: Option<OrchestrationPlan>,
    title: Option<&str>,
    goal: Option<&str>,
    phases: &[PlanPhaseInput],
) -> Result<OrchestrationPlan, String> {
    let phases: Vec<(&PlanPhaseInput, Option<&PhaseGateInput>)> =
        phases.iter().map(|phase| (phase, None)).collect();
    apply_plan_inputs(existing, title, goal, &phases)
}

/// Declare a plan, or revise the stored one.
///
/// A phase whose id is kept keeps its agents, status, hand-off and timestamps;
/// its title, goal, kind and dependencies take the new values, and its gate
/// the one declared (a gate left out stays as it is). A phase that is dropped
/// from the list is removed only when nothing was ever spawned into it,
/// because its children would otherwise vanish from every surface that draws
/// the plan.
pub(crate) fn apply_gated_plan(
    existing: Option<OrchestrationPlan>,
    title: Option<&str>,
    goal: Option<&str>,
    phases: &[OrchestratePhaseInput],
) -> Result<OrchestrationPlan, String> {
    let phases: Vec<(&PlanPhaseInput, Option<&PhaseGateInput>)> = phases
        .iter()
        .map(|input| (&input.phase, input.gate.as_ref()))
        .collect();
    apply_plan_inputs(existing, title, goal, &phases)
}

fn apply_plan_inputs(
    existing: Option<OrchestrationPlan>,
    title: Option<&str>,
    goal: Option<&str>,
    phases: &[(&PlanPhaseInput, Option<&PhaseGateInput>)],
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
    for (input, gate) in phases {
        let phase = build_phase(input, *gate, &seen, existing.as_ref())?;
        seen.push(input.id.trim());
        next_phases.push(phase);
    }

    if let Some(previous) = &existing {
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
    let open: Vec<&str> = previous
        .phases
        .iter()
        .filter(|phase| !phase.status.is_closed())
        .map(|phase| phase.id.as_str())
        .collect();
    if !open.is_empty() {
        return Err(format!(
            "fresh:true replaces a plan only once every phase is completed or skipped; \
             still open: {}. Close them with action:\"complete\" (status:\"skipped\" for \
             work you will not do), or revise this plan without fresh. Nothing was changed.",
            join_ids(open.into_iter())
        ));
    }
    let title = title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or(previous.title.as_str());
    apply_gated_plan(None, Some(title), goal, phases)
}

/// `action:"start"`: a pending or blocked phase moves to in progress. Returns
/// the plan and a sentence for the model.
pub(crate) fn apply_start(
    plan: Option<OrchestrationPlan>,
    phase_id: &str,
    now: DateTime<Utc>,
) -> Result<(OrchestrationPlan, String), String> {
    let mut plan = plan.ok_or_else(|| NO_PLAN.to_string())?;
    let unknown = plan.unknown_phase(phase_id);
    let not_completed: Vec<String> = {
        let phase = plan.phase(phase_id).ok_or(unknown)?;
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
pub(crate) struct PhaseAgentState {
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

/// `action:"complete"`: close a phase. A BARRIER: refused while any child in
/// the phase is still queued, running or being cancelled.
///
/// `checked_agents` is the set of child ids whose states were gathered before
/// the write began. A child recorded into the phase after that (a spawn that
/// raced this call) was never checked, so the call is refused rather than
/// closing the phase over a child nobody looked at.
pub(crate) fn apply_complete(
    plan: Option<OrchestrationPlan>,
    phase_id: &str,
    status: PhaseStatus,
    summary: Option<&str>,
    agents: &[PhaseAgentState],
    now: DateTime<Utc>,
) -> Result<(OrchestrationPlan, String), String> {
    let mut plan = plan.ok_or_else(|| NO_PLAN.to_string())?;
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
        .map(|phase| {
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
                gate: gate_snapshot(phase),
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
        }
    }
    text
}

/// A gate's state in a few words: "2 questions at P(yes) >= 0.80, attempt
/// 1 of 3, last failed (lowest P(yes) 0.42)".
pub fn gate_words(gate: &PhaseGateSnapshot) -> String {
    let mut words = format!(
        "{} question{} at P(yes) >= {:.2}, {} of {} attempts used",
        gate.question_count,
        if gate.question_count == 1 { "" } else { "s" },
        gate.threshold,
        gate.attempts,
        gate.max_attempts
    );
    match (gate.last_passed, gate.last_min_p) {
        (Some(true), _) => words.push_str(", passed"),
        (Some(false), Some(p)) => words.push_str(&format!(", last failed (lowest P(yes) {p:.2})")),
        (Some(false), None) => words.push_str(", last failed"),
        (None, _) => {}
    }
    if gate.blocked {
        words.push_str(", blocked");
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
                ", gated (attempt {} of {}{})",
                gate.attempts,
                gate.max_attempts,
                if gate.blocked { ", blocked" } else { "" }
            ));
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
/// phase, or one already completed or skipped. `Err` carries the sentence the
/// model reads.
pub async fn validate_spawn_phase(
    session_manager: &SessionManager,
    parent_session_id: &str,
    phase_id: &str,
) -> Result<(), String> {
    let plan = load_plan(session_manager, parent_session_id)
        .await
        .map_err(|e| {
            format!("Could not read this chat's orchestration plan: {e}. Nothing was created.")
        })?;
    check_spawn_phase(plan.as_ref(), phase_id)
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
    if plan.agent_count() >= MAX_PLAN_AGENTS {
        return Err(format!(
            "This plan already has {MAX_PLAN_AGENTS} subagents, its limit. For further \
             phased work, complete or skip its open phases ({}), then start a new plan with \
             workspace_orchestrate action:\"plan\" fresh:true. Or spawn without a phase. \
             Nothing was created.",
            plan.open_phase_ids()
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
    let unknown = plan.unknown_phase(phase_id);
    let phase = plan.phase_mut(phase_id).ok_or(unknown)?;
    // The spawn checked the phase before its child existed; a `complete` may
    // have closed it since. A closed phase never takes another child.
    if phase.status.is_closed() {
        return Err(format!(
            "Phase \"{phase_id}\" was {} while this subagent was being created, so nothing \
             more can be spawned into it.",
            phase.status.as_str()
        ));
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

fn gate_snapshot(phase: &PlanPhase) -> Option<PhaseGateSnapshot> {
    let gate = phase.gate.as_ref()?;
    let state = phase.gate_state.clone().unwrap_or_default();
    Some(PhaseGateSnapshot {
        threshold: gate.threshold,
        question_count: u32::try_from(gate.questions.len()).unwrap_or(u32::MAX),
        attempts: state.attempts,
        max_attempts: gate.max_attempts,
        last_passed: state.last_passed,
        last_min_p: state.last_min_p,
        blocked: gate_blocked(phase),
    })
}

/// The refusal for completing a phase whose gate has no attempts left, or
/// `None` when the gate may still be asked.
pub(crate) fn refuse_exhausted_gate(phase: &PlanPhase) -> Option<String> {
    if !gate_exhausted(phase) {
        return None;
    }
    let gate = phase.gate.as_ref()?;
    Some(format!(
        "Phase \"{}\"'s gate failed all {} attempts, so the phase cannot be completed through \
         it. Tell the user what the gate asked and what failed. With their agreement, declare a \
         different gate for the phase with action:\"plan\" (a changed gate starts its attempts \
         again), or close the phase with status:\"skipped\". Nothing was changed.",
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

/// Ask the gate's questions of `state` through `service`, as the chat
/// `session` describes (origin `gate`, the phase id and title, the turn).
pub(crate) async fn evaluate_gate(
    service: &DecisionService,
    session: &DecisionSession,
    gate: &PhaseGate,
    state: String,
) -> GateVerdict {
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
    let request = DecisionRequest {
        questions,
        context: DecisionContext::Text(state),
    };
    let decisions = match service
        .decide(Some(session), gate.model.as_ref(), &request)
        .await
    {
        Ok(decisions) => decisions,
        Err(error) => return GateVerdict::Error(error.to_string()),
    };
    if !gate.allow_uncalibrated {
        if let Some(uncalibrated) = decisions.iter().find(|d| !d.calibrated) {
            return GateVerdict::Error(format!(
                "{} returned uncalibrated probabilities, which a gate refuses. Use a decision \
                 model with its own probabilities, or set allowUncalibrated on the gate to \
                 accept them.",
                uncalibrated.model.label()
            ));
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
    if answers.iter().all(|answer| answer.passed) {
        GateVerdict::Passed(answers)
    } else {
        GateVerdict::Failed(answers)
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
/// while the gate was being evaluated, or (on a pass) when the barrier no
/// longer holds.
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
        let (mut plan, note) = apply_complete(
            Some(plan),
            phase_id,
            PhaseStatus::Completed,
            summary,
            agents,
            now,
        )?;
        let phase = plan.phase_mut(phase_id).expect("completed above");
        let state = phase.gate_state.get_or_insert_with(GateState::default);
        state.attempts += 1;
        state.consecutive_errors = 0;
        state.last_passed = Some(true);
        state.last_min_p = min_p(answers);
        state.last_error = None;
        state.last_at = Some(now);
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
             questions that fell short:{failing}\nTell the user what the gate asked and what the \
             results lacked. With their agreement, declare a different gate with \
             action:\"plan\" (a changed gate starts its attempts again), or close the phase \
             with status:\"skipped\"."
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
             user; once a decision model this chat may use is reachable, complete the phase \
             again."
        );
    }
    format!(
        "Phase \"{phase_id}\" was NOT completed: its gate could not be evaluated ({message}). \
         Nothing was decided and no attempt was used. Complete the phase again to retry; a \
         second failure in a row blocks it."
    )
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
        }
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

    /// A revision that lists the phases again without their gates keeps them;
    /// a changed gate starts its attempts again.
    #[test]
    fn a_revision_keeps_a_gate_left_out_and_a_changed_gate_starts_over() {
        let mut plan = gated_plan();
        let gate = stored_gate(&plan);
        plan.phase_mut("build").unwrap().gate_state = Some(GateState {
            attempts: 2,
            last_passed: Some(false),
            ..GateState::default()
        });

        let kept = apply_gated_plan(
            Some(plan.clone()),
            None,
            None,
            &[
                gated("research", "research", &[], None),
                gated("build", "implement", &["research"], None),
            ],
        )
        .unwrap();
        let build = kept.phase("build").unwrap();
        assert_eq!(build.gate.as_ref(), Some(&gate));
        assert_eq!(build.gate_state.as_ref().unwrap().attempts, 2);

        let same = apply_gated_plan(
            Some(plan.clone()),
            None,
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
        .unwrap();
        assert_eq!(
            same.phase("build")
                .unwrap()
                .gate_state
                .as_ref()
                .unwrap()
                .attempts,
            2,
            "the same gate declared again keeps its record"
        );

        let changed = apply_gated_plan(
            Some(plan),
            None,
            None,
            &[
                gated("research", "research", &[], None),
                gated(
                    "build",
                    "implement",
                    &["research"],
                    Some(PhaseGateInput {
                        threshold: Some(0.6),
                        ..gate_input(&[("tests_pass", "Do the tests pass?")])
                    }),
                ),
            ],
        )
        .unwrap();
        let build = changed.phase("build").unwrap();
        assert_eq!(build.gate.as_ref().unwrap().threshold, 0.6);
        assert_eq!(build.gate_state, None, "a changed gate starts over");
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
        assert!(note.contains("Tell the user"), "{note}");
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
    }
}
