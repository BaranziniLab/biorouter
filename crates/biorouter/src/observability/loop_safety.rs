//! BR-67: runtime observability for loop-safety events.
//!
//! Waves 0–2 gave the agent a stack of loop guards — the staged repetition stop
//! (BR-29), the near-duplicate / oscillation detector (BR-30), the
//! repeated-failing-result guard (BR-31), the periodic stall check (BR-32), the
//! per-reply budget (BR-35), the mistake streak (BR-66), and the PreToolUse hook
//! veto. Each of them can silently end a turn, and until now none of them left a
//! machine-readable trace: an operator could not tell *why* the agent stopped,
//! nor whether the guards fire at all in practice.
//!
//! This module is the one emit point they all funnel through. It does three
//! things, all observe-only — nothing here can change what the agent does:
//!
//! 1. logs a structured `tracing` event on the `loop_safety` target (level
//!    `WARN` when the agent was stopped, `INFO` when it was merely nudged), so
//!    the events land in the existing file/stdout logging (`crate::logging`);
//! 2. bumps a per-kind counter, so `counters()` answers "how often did each
//!    guard fire in this process" without a log scrape;
//! 3. fans the event out to any registered [`LoopSafetyObserver`] (the GUI, a
//!    session recorder, a test).
//!
//! **Redaction-safe by construction.** A [`LoopSafetyEvent`] can only carry
//! names, stable codes and counts — a tool *name*, a finding id, a repeat count,
//! a budget axis. There is no field that can hold tool arguments, tool output,
//! model prose, or the stall judge's reason text, so an emit site cannot leak
//! user content into a log even by accident. That is the invariant the internal
//! review asked for (`verification.md` #8), and it is enforced by the type, not
//! by convention.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use super::ObsEvent;
use crate::config::Config;

/// Config key: set to `false` to silence loop-safety emission entirely.
pub const LOOP_SAFETY_TRACE_KEY: &str = "BIOROUTER_LOOP_SAFETY_TRACE";

/// `tracing` target every loop-safety event is logged on, so it can be filtered
/// in or out on its own (`RUST_LOG=loop_safety=info`).
pub const LOOP_SAFETY_TARGET: &str = "loop_safety";

/// Which loop-safety mechanism fired, and what it did.
///
/// The `*Stop` / `HookBlock` / `Cancelled` variants mean the agent was actually
/// prevented from doing something; the rest mean it was warned or nudged and
/// carried on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopSafetyKind {
    /// BR-29/30: a repetition guard warned; the call still ran.
    RepetitionWarn,
    /// BR-29/30/31: a repetition guard denied the call.
    RepetitionStop,
    /// BR-31: a tool keeps failing the same way; nudge at the result seam.
    FailureLoopNudge,
    /// BR-66: consecutive failed tool calls of any kind; reflect-and-replan nudge.
    MistakeStreakNudge,
    /// BR-66: a recoverable provider error earned one more attempt with a hint.
    ProviderErrorRecover,
    /// BR-66: the recoverable-provider-error budget ran out; the reply stopped.
    ProviderErrorStop,
    /// BR-32: the stall check flagged a loop; the model was nudged.
    StallNudge,
    /// BR-32: the stall check gave up; the model was told to wrap up.
    StallGiveUp,
    /// BR-32: the wrap-up instruction was ignored; the turn was ended.
    StallStop,
    /// BR-35: the per-reply budget is running low.
    BudgetWarn,
    /// BR-35: the per-reply budget is spent; the model was told to wrap up.
    BudgetExceeded,
    /// BR-35: the wrap-up instruction was ignored; the reply was ended.
    BudgetStop,
    /// The per-turn action ceiling (`max_turns`) ended the turn.
    TurnLimitStop,
    /// The per-turn tool-call ceiling (`max_tool_calls`) ended the turn.
    ToolCallLimitStop,
    /// A PreToolUse hook denied the call.
    HookBlock,
    /// A PreToolUse hook escalated the call to the user.
    HookAsk,
    /// The turn was cancelled (user interrupt / shutdown).
    Cancelled,
    /// BR-47: a post-edit syntax check flagged the just-written file; the model
    /// was nudged with the diagnostics at the result seam.
    PostEditDiagnostics,
    /// BR-50: the optional self-critique pass flagged a possible defect in an
    /// ordinary answer; the model was asked to revise before finishing.
    SelfCritiqueRevise,
    /// BR-48: the interactive done-ness gate's checks failed; the model was
    /// asked to keep working before it could finish.
    DoneGateBlock,
    /// BR-48: the done-ness gate spent its attempt budget with checks still
    /// failing; the turn was allowed to finish anyway.
    DoneGateGiveUp,
    /// A per-chat, default per-chat or monthly usage limit is at 80% or more.
    UsageLimitWarn,
    /// A usage limit is reached; the turn ended before the next model request.
    UsageLimitStop,
}

/// Every kind, in declaration order. The counter table is indexed by position
/// here, so a new variant must be appended to both.
pub const ALL_KINDS: [LoopSafetyKind; 23] = [
    LoopSafetyKind::RepetitionWarn,
    LoopSafetyKind::RepetitionStop,
    LoopSafetyKind::FailureLoopNudge,
    LoopSafetyKind::MistakeStreakNudge,
    LoopSafetyKind::ProviderErrorRecover,
    LoopSafetyKind::ProviderErrorStop,
    LoopSafetyKind::StallNudge,
    LoopSafetyKind::StallGiveUp,
    LoopSafetyKind::StallStop,
    LoopSafetyKind::BudgetWarn,
    LoopSafetyKind::BudgetExceeded,
    LoopSafetyKind::BudgetStop,
    LoopSafetyKind::TurnLimitStop,
    LoopSafetyKind::ToolCallLimitStop,
    LoopSafetyKind::HookBlock,
    LoopSafetyKind::HookAsk,
    LoopSafetyKind::Cancelled,
    LoopSafetyKind::PostEditDiagnostics,
    LoopSafetyKind::SelfCritiqueRevise,
    LoopSafetyKind::DoneGateBlock,
    LoopSafetyKind::DoneGateGiveUp,
    LoopSafetyKind::UsageLimitWarn,
    LoopSafetyKind::UsageLimitStop,
];

impl LoopSafetyKind {
    /// Stable snake_case name, used as the log field, the counter key, and the
    /// guardrail span name.
    pub fn as_str(self) -> &'static str {
        match self {
            LoopSafetyKind::RepetitionWarn => "repetition_warn",
            LoopSafetyKind::RepetitionStop => "repetition_stop",
            LoopSafetyKind::FailureLoopNudge => "failure_loop_nudge",
            LoopSafetyKind::MistakeStreakNudge => "mistake_streak_nudge",
            LoopSafetyKind::ProviderErrorRecover => "provider_error_recover",
            LoopSafetyKind::ProviderErrorStop => "provider_error_stop",
            LoopSafetyKind::StallNudge => "stall_nudge",
            LoopSafetyKind::StallGiveUp => "stall_give_up",
            LoopSafetyKind::StallStop => "stall_stop",
            LoopSafetyKind::BudgetWarn => "budget_warn",
            LoopSafetyKind::BudgetExceeded => "budget_exceeded",
            LoopSafetyKind::BudgetStop => "budget_stop",
            LoopSafetyKind::TurnLimitStop => "turn_limit_stop",
            LoopSafetyKind::ToolCallLimitStop => "tool_call_limit_stop",
            LoopSafetyKind::HookBlock => "hook_block",
            LoopSafetyKind::HookAsk => "hook_ask",
            LoopSafetyKind::Cancelled => "cancelled",
            LoopSafetyKind::PostEditDiagnostics => "post_edit_diagnostics",
            LoopSafetyKind::SelfCritiqueRevise => "self_critique_revise",
            LoopSafetyKind::DoneGateBlock => "done_gate_block",
            LoopSafetyKind::DoneGateGiveUp => "done_gate_give_up",
            LoopSafetyKind::UsageLimitWarn => "usage_limit_warn",
            LoopSafetyKind::UsageLimitStop => "usage_limit_stop",
        }
    }

    /// Did this event *stop* the agent (rather than warn or nudge it)? Drives
    /// the log level and the `blocked` flag of the guardrail span.
    pub fn is_stop(self) -> bool {
        matches!(
            self,
            LoopSafetyKind::RepetitionStop
                | LoopSafetyKind::ProviderErrorStop
                | LoopSafetyKind::StallStop
                | LoopSafetyKind::BudgetStop
                | LoopSafetyKind::UsageLimitStop
                | LoopSafetyKind::TurnLimitStop
                | LoopSafetyKind::ToolCallLimitStop
                | LoopSafetyKind::HookBlock
                | LoopSafetyKind::Cancelled
        )
    }

    fn index(self) -> usize {
        ALL_KINDS
            .iter()
            .position(|kind| *kind == self)
            .expect("every kind is in ALL_KINDS")
    }
}

/// One loop-safety decision.
///
/// Every field is a name, a stable code, or a number — see the module docs: the
/// struct has nowhere to put tool arguments or message text, which is what makes
/// emission safe to log unconditionally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopSafetyEvent {
    pub kind: LoopSafetyKind,
    /// The session the guard fired in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The tool whose call tripped the guard — its *name*, never its arguments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// Stable finding id of the guard that fired (e.g. `REP-004`), when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finding_id: Option<String>,
    /// How many times the thing that tripped the guard happened: identical calls
    /// in a row, failures in a row, actions taken this turn, retry attempt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    /// The threshold `count` was measured against, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Which budget axis tripped (`seconds` / `tokens` / `usd`), for BR-35.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis: Option<String>,
}

impl LoopSafetyEvent {
    pub fn new(kind: LoopSafetyKind) -> Self {
        Self {
            kind,
            session_id: None,
            tool: None,
            finding_id: None,
            count: None,
            limit: None,
            axis: None,
        }
    }

    pub fn session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn tool(mut self, tool: impl Into<String>) -> Self {
        self.tool = Some(tool.into());
        self
    }

    pub fn finding_id(mut self, finding_id: impl Into<String>) -> Self {
        self.finding_id = Some(finding_id.into());
        self
    }

    pub fn count(mut self, count: u32) -> Self {
        self.count = Some(count);
        self
    }

    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn axis(mut self, axis: impl Into<String>) -> Self {
        self.axis = Some(axis.into());
        self
    }

    /// Set the axis when the budget knows which one tripped (it is `None` for a
    /// snapshot taken before any limit was crossed).
    pub fn maybe_axis(self, axis: Option<&str>) -> Self {
        match axis {
            Some(axis) => self.axis(axis),
            None => self,
        }
    }

    /// The same decision as a span in the [`super::TraceBuilder`] model: a
    /// guardrail, `blocked` when the agent was actually stopped.
    pub fn as_obs_event(&self) -> ObsEvent {
        ObsEvent::Guardrail {
            name: self.kind.as_str().to_string(),
            blocked: self.kind.is_stop(),
        }
    }
}

/// A sink for loop-safety events (GUI stream, session recorder, test spy).
pub trait LoopSafetyObserver: Send + Sync {
    fn on_loop_safety_event(&self, event: &LoopSafetyEvent);
}

fn observers() -> &'static RwLock<Vec<Arc<dyn LoopSafetyObserver>>> {
    static OBSERVERS: OnceLock<RwLock<Vec<Arc<dyn LoopSafetyObserver>>>> = OnceLock::new();
    OBSERVERS.get_or_init(|| RwLock::new(Vec::new()))
}

/// Register a sink. Observers are never removed; the process-lifetime sinks this
/// is for (a GUI stream, a recorder) outlive any one session.
pub fn subscribe(observer: Arc<dyn LoopSafetyObserver>) {
    if let Ok(mut sinks) = observers().write() {
        sinks.push(observer);
    }
}

fn counters_table() -> &'static [AtomicU64; ALL_KINDS.len()] {
    static COUNTERS: OnceLock<[AtomicU64; ALL_KINDS.len()]> = OnceLock::new();
    COUNTERS.get_or_init(|| std::array::from_fn(|_| AtomicU64::new(0)))
}

/// Is emission on? Resolved once (a config read touches the filesystem) and
/// defaulted on: this is observe-only, so it cannot change agent behavior.
fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        Config::global()
            .get_param::<bool>(LOOP_SAFETY_TRACE_KEY)
            .unwrap_or(true)
    })
}

/// Record one loop-safety decision: log it, count it, fan it out.
///
/// Cheap and infallible — a guard tripping is rare, and an observer panicking or
/// a poisoned lock must never take down a turn.
pub fn emit(event: LoopSafetyEvent) {
    if !enabled() {
        return;
    }

    counters_table()[event.kind.index()].fetch_add(1, Ordering::Relaxed);

    let kind = event.kind.as_str();
    let session_id = event.session_id.as_deref().unwrap_or_default();
    let tool = event.tool.as_deref().unwrap_or_default();
    let finding_id = event.finding_id.as_deref().unwrap_or_default();
    let count = event.count.unwrap_or_default();
    let limit = event.limit.unwrap_or_default();
    let axis = event.axis.as_deref().unwrap_or_default();
    if event.kind.is_stop() {
        tracing::warn!(
            target: LOOP_SAFETY_TARGET,
            kind, session_id, tool, finding_id, count, limit, axis,
            "loop-safety guard stopped the agent"
        );
    } else {
        tracing::info!(
            target: LOOP_SAFETY_TARGET,
            kind, session_id, tool, finding_id, count, limit, axis,
            "loop-safety guard nudged the agent"
        );
    }

    let sinks = match observers().read() {
        Ok(sinks) => sinks.clone(),
        Err(_) => return,
    };
    for sink in sinks {
        sink.on_loop_safety_event(&event);
    }
}

/// How many times each guard has fired in this process, keyed by
/// [`LoopSafetyKind::as_str`]. Kinds that never fired are omitted.
pub fn counters() -> BTreeMap<&'static str, u64> {
    let table = counters_table();
    ALL_KINDS
        .iter()
        .enumerate()
        .filter_map(|(index, kind)| {
            let count = table[index].load(Ordering::Relaxed);
            (count > 0).then_some((kind.as_str(), count))
        })
        .collect()
}

/// Zero the counters (a fresh benchmark run, a test).
pub fn reset_counters() {
    for counter in counters_table() {
        counter.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The counters and the observer list are process-global, so the tests that
    /// touch them must not interleave.
    static GLOBAL: Mutex<()> = Mutex::new(());

    #[derive(Default)]
    struct Spy {
        seen: Mutex<Vec<LoopSafetyEvent>>,
    }

    impl LoopSafetyObserver for Spy {
        fn on_loop_safety_event(&self, event: &LoopSafetyEvent) {
            self.seen.lock().unwrap().push(event.clone());
        }
    }

    #[test]
    fn every_kind_has_a_unique_name_and_slot() {
        let mut names: Vec<&str> = ALL_KINDS.iter().map(|kind| kind.as_str()).collect();
        names.sort_unstable();
        let unique = names.len();
        names.dedup();
        assert_eq!(unique, names.len(), "kind names must be unique");

        for (index, kind) in ALL_KINDS.iter().enumerate() {
            assert_eq!(kind.index(), index, "counter slot must match declaration");
        }
    }

    #[test]
    fn stops_are_distinguished_from_nudges() {
        assert!(LoopSafetyKind::RepetitionStop.is_stop());
        assert!(LoopSafetyKind::BudgetStop.is_stop());
        assert!(LoopSafetyKind::HookBlock.is_stop());
        assert!(LoopSafetyKind::Cancelled.is_stop());
        assert!(!LoopSafetyKind::RepetitionWarn.is_stop());
        assert!(!LoopSafetyKind::StallNudge.is_stop());
        assert!(!LoopSafetyKind::BudgetWarn.is_stop());
        assert!(LoopSafetyKind::UsageLimitStop.is_stop());
        assert!(!LoopSafetyKind::UsageLimitWarn.is_stop());
        assert!(!LoopSafetyKind::HookAsk.is_stop());
    }

    #[test]
    fn a_stop_becomes_a_blocked_guardrail_span() {
        let event = LoopSafetyEvent::new(LoopSafetyKind::RepetitionStop)
            .tool("shell")
            .finding_id("REP-001")
            .count(3);
        assert_eq!(
            event.as_obs_event(),
            ObsEvent::Guardrail {
                name: "repetition_stop".to_string(),
                blocked: true,
            }
        );

        let warn = LoopSafetyEvent::new(LoopSafetyKind::RepetitionWarn).tool("shell");
        assert_eq!(
            warn.as_obs_event(),
            ObsEvent::Guardrail {
                name: "repetition_warn".to_string(),
                blocked: false,
            }
        );
    }

    #[test]
    fn serialization_carries_names_and_counts_only() {
        let event = LoopSafetyEvent::new(LoopSafetyKind::BudgetStop)
            .session("s1")
            .tool("developer__shell")
            .count(42)
            .limit(40)
            .axis("tokens");
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["kind"], "budget_stop");
        assert_eq!(json["tool"], "developer__shell");
        assert_eq!(json["count"], 42);
        assert_eq!(json["axis"], "tokens");
        // No field exists that could hold arguments or prose: the object is
        // exactly the redaction-safe surface.
        // Sorted: serde_json orders map keys alphabetically (BTreeMap) only when
        // `preserve_order` is off. `biorouter-server` enables it, and cargo unifies
        // features workspace-wide, so under a full-workspace build the order is
        // insertion order instead. The invariant here is the key SET, not its order.
        let mut keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["axis", "count", "kind", "limit", "session_id", "tool"]
        );
    }

    #[test]
    fn unset_fields_are_omitted() {
        let json = serde_json::to_value(LoopSafetyEvent::new(LoopSafetyKind::Cancelled)).unwrap();
        assert_eq!(json.as_object().unwrap().len(), 1);
        assert_eq!(json["kind"], "cancelled");
    }

    #[test]
    fn emit_counts_and_fans_out() {
        // The counters are process-global and the repetition guard's own tests
        // emit into them from other threads, so this test asserts only on kinds
        // nothing else in the lib test binary emits.
        let _guard = GLOBAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_counters();
        let spy = Arc::new(Spy::default());
        subscribe(spy.clone());

        emit(
            LoopSafetyEvent::new(LoopSafetyKind::TurnLimitStop)
                .session("obs-emit-test")
                .count(51)
                .limit(50),
        );
        emit(LoopSafetyEvent::new(LoopSafetyKind::Cancelled).session("obs-emit-test"));
        emit(LoopSafetyEvent::new(LoopSafetyKind::Cancelled).session("obs-emit-test"));

        let counts = counters();
        assert_eq!(counts.get("turn_limit_stop"), Some(&1));
        assert_eq!(counts.get("cancelled"), Some(&2));
        assert_eq!(
            counts.get("stall_nudge"),
            None,
            "a guard that never fired is omitted"
        );

        let seen: Vec<LoopSafetyEvent> = spy
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.session_id.as_deref() == Some("obs-emit-test"))
            .cloned()
            .collect();
        assert_eq!(seen.len(), 3, "every event reaches the observer");
        assert_eq!(seen[0].kind, LoopSafetyKind::TurnLimitStop);
        assert_eq!(seen[0].limit, Some(50));
        assert_eq!(seen[2].kind, LoopSafetyKind::Cancelled);

        reset_counters();
        assert_eq!(counters().get("cancelled"), None, "reset zeroes the table");
    }
}
