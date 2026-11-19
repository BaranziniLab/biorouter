//! Why the last reply loop on an agent ended, for a caller that did not watch
//! it live: a headless `biorouter run` reporting its end in the stream-json
//! `complete` event, the `json` document and its exit status.
//!
//! [`TurnExit`](super::agent) answers a different question (may a steer the
//! person types resume the turn?) with two values, and the server consumes it.
//! Several different endings share each of its values: an action limit, a
//! tool-call limit, a stall stop and a monitor's stop are all resumable, and a
//! normal end, a spend cap, a usage limit and an abort are all final. So the
//! reason is recorded beside it, at each site that ends the loop, and read
//! without being consumed ([`super::Agent::last_turn_stop`]). It is not derived
//! from the loop-safety trace, which can be switched off.
//!
//! A cap stop stays what it was in the agent loop: an ordinary, resumable end
//! of the turn, never a [`super::turn_abort::TurnAbortCode`]. Only what a
//! headless caller reports about it changes.

/// How a reply loop ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStopReason {
    /// The model ended its turn (no tool call, every exit gate let it finish),
    /// or a structured final output ended it.
    EndTurn,
    /// `max_turns`: the action limit without user input.
    TurnLimit,
    /// `max_tool_calls`: the per-reply tool-call limit.
    ToolCallLimit,
    /// The stall check told the model to wrap up and it kept going.
    StallStop,
    /// The per-reply budget (wall clock, tokens, dollars) was spent and the
    /// wrap-up grace ran out.
    BudgetStop,
    /// A usage limit was reached, or the next request was estimated to pass
    /// one.
    UsageLimitStop,
    /// A decision-model monitor stopped the turn.
    MonitorStop,
    /// The turn aborted ([`super::turn_abort::TurnAbortCode`]); the code is
    /// reported separately.
    Aborted,
    /// The turn's cancellation token tripped.
    Cancelled,
}

impl TurnStopReason {
    /// The `stop_reason` a headless run reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EndTurn => "end_turn",
            Self::TurnLimit => "turn_limit",
            Self::ToolCallLimit => "tool_call_limit",
            Self::StallStop => "stall_stop",
            Self::BudgetStop => "budget_stop",
            Self::UsageLimitStop => "usage_limit_stop",
            Self::MonitorStop => "monitor_stop",
            Self::Aborted => "aborted",
            Self::Cancelled => "cancelled",
        }
    }

    /// A limit stopped the work before it was done: the action limit, the
    /// tool-call limit, the stall stop, the budget or a usage limit. A monitor's
    /// stop is the chat's supervision and is reported on its own.
    pub fn is_limit_stop(self) -> bool {
        matches!(
            self,
            Self::TurnLimit
                | Self::ToolCallLimit
                | Self::StallStop
                | Self::BudgetStop
                | Self::UsageLimitStop
        )
    }
}

/// What the last reply loop recorded as it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnStopRecord {
    pub reason: TurnStopReason,
    /// Model requests the loop made (its completed iterations, length
    /// continuations included). Not provider retries, and not Biorouter's own
    /// side requests (a title, a summary, a compaction).
    pub actions: u32,
    /// Tool calls the loop dispatched.
    pub tool_calls: u32,
    /// The limit that stopped it, for [`TurnStopReason::TurnLimit`] and
    /// [`TurnStopReason::ToolCallLimit`].
    pub limit: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_names_are_snake_case_and_only_limits_count_as_limit_stops() {
        let all = [
            (TurnStopReason::EndTurn, "end_turn", false),
            (TurnStopReason::TurnLimit, "turn_limit", true),
            (TurnStopReason::ToolCallLimit, "tool_call_limit", true),
            (TurnStopReason::StallStop, "stall_stop", true),
            (TurnStopReason::BudgetStop, "budget_stop", true),
            (TurnStopReason::UsageLimitStop, "usage_limit_stop", true),
            (TurnStopReason::MonitorStop, "monitor_stop", false),
            (TurnStopReason::Aborted, "aborted", false),
            (TurnStopReason::Cancelled, "cancelled", false),
        ];
        for (reason, wire, limit) in all {
            assert_eq!(reason.as_str(), wire);
            assert_eq!(reason.is_limit_stop(), limit, "{wire}");
        }
    }
}
