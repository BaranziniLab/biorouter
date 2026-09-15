//! What the reply loop is waiting on right now, readable from outside the loop.
//!
//! A turn that "froze after a steer" is indistinguishable, from the outside,
//! from one that is legitimately waiting on a 45 s shell call, an approval
//! card, a child agent or the provider's first byte — and the reply loop's
//! generator is the one place that knows which. This cell is that knowledge,
//! published: the loop stamps a phase at each of its await sites, the
//! `/interrupt` route logs the phase a steer was accepted into, and
//! `GET /sessions/running` serves it, so a stall can be named instead of
//! guessed at.
//!
//! ⚠ It lives ON the [`Agent`](crate::agents::Agent), in atomics, and never as a
//! local of `reply_internal`'s generator: that generator's debug `poll` frame is
//! within a few percent of overflowing the stack during delegation, and every
//! local it holds across a `yield` is paid for 2–3 times over.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// One await site class in the reply loop. `Idle` means no loop is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum LoopPhase {
    Idle = 0,
    /// Between the turn lock and the loop's first step: hooks, context.
    Prologue = 1,
    /// Summarising the history before the first model call.
    AutoCompaction = 2,
    /// Waiting for the provider to answer the request (time-to-first-byte).
    ProviderOpen = 3,
    /// Reading the provider's streamed answer.
    Streaming = 4,
    /// Inspecting and permission-checking the model's tool calls.
    Gating = 5,
    /// A tool call is parked on a card that needs the person.
    ApprovalWait = 6,
    /// Tool calls are running.
    ToolBatch = 7,
    /// A live-steering provider has not yet acknowledged a steer.
    LiveAckWait = 8,
    /// Done gate, self-critique, Stop hooks.
    ExitGates = 9,
    /// The loop has stopped and is waiting for delegated children to report.
    SupervisionWait = 10,
    /// Persisting what the turn leaves behind.
    Settling = 11,
}

impl LoopPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Prologue => "prologue",
            Self::AutoCompaction => "auto_compaction",
            Self::ProviderOpen => "provider_open",
            Self::Streaming => "streaming",
            Self::Gating => "gating",
            Self::ApprovalWait => "approval_wait",
            Self::ToolBatch => "tool_batch",
            Self::LiveAckWait => "live_ack_wait",
            Self::ExitGates => "exit_gates",
            Self::SupervisionWait => "supervision_wait",
            Self::Settling => "settling",
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Prologue,
            2 => Self::AutoCompaction,
            3 => Self::ProviderOpen,
            4 => Self::Streaming,
            5 => Self::Gating,
            6 => Self::ApprovalWait,
            7 => Self::ToolBatch,
            8 => Self::LiveAckWait,
            9 => Self::ExitGates,
            10 => Self::SupervisionWait,
            11 => Self::Settling,
            _ => Self::Idle,
        }
    }
}

impl std::fmt::Display for LoopPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// The phase and the moment it was entered. Two atomics, so a reader can see a
/// phase paired with the previous phase's timestamp for one instruction; the
/// age is diagnostic, and that skew is not worth a lock on the loop's hot path.
#[derive(Debug, Default)]
pub struct LoopPhaseCell {
    phase: AtomicU8,
    entered_ms: AtomicU64,
}

impl LoopPhaseCell {
    /// Enter `phase`. A re-entry of the phase already current keeps its original
    /// timestamp, so a loop that stamps the same phase on every poll still
    /// reports how long it has really been there.
    pub fn enter(&self, phase: LoopPhase) {
        let previous = self.phase.swap(phase as u8, Ordering::AcqRel);
        if previous != phase as u8 {
            self.entered_ms.store(now_ms(), Ordering::Release);
        }
    }

    /// The current phase and how long ago it was entered, in milliseconds.
    pub fn snapshot(&self) -> (LoopPhase, u64) {
        let phase = LoopPhase::from_u8(self.phase.load(Ordering::Acquire));
        let entered = self.entered_ms.load(Ordering::Acquire);
        let age = if entered == 0 {
            0
        } else {
            now_ms().saturating_sub(entered)
        };
        (phase, age)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn re_entering_a_phase_keeps_its_age_and_a_new_phase_restarts_it() {
        let cell = LoopPhaseCell::default();
        assert_eq!(cell.snapshot().0, LoopPhase::Idle);
        cell.enter(LoopPhase::ToolBatch);
        cell.entered_ms.store(now_ms() - 5_000, Ordering::Release);
        cell.enter(LoopPhase::ToolBatch);
        assert!(
            cell.snapshot().1 >= 5_000,
            "a re-entry must not reset the age"
        );
        cell.enter(LoopPhase::Streaming);
        let (phase, age) = cell.snapshot();
        assert_eq!(phase, LoopPhase::Streaming);
        assert!(age < 5_000);
    }
}
