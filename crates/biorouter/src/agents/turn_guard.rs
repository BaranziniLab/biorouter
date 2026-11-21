//! Structural enforcement for repetition-policy denials.
//!
//! [`crate::tool_monitor::RepetitionInspector`] owns the policy: it decides
//! whether a call should warn, run, or be denied. This module deliberately has
//! no threshold and never counts attempts. It records each valid denied exact
//! `(tool, arguments)` signature, permits one model feedback opportunity, and
//! converts every later valid denial into the typed terminal event consumed by
//! the CLI, server, desktop, and Apps runtime.
//!
//! The guard is created once per user reply. Ending the reply after the terminal
//! denied tool response is recorded makes the stop structural: no additional
//! billed provider call is needed, and a new user message starts with a fresh
//! guard.

use std::collections::HashSet;

use crate::conversation::message::ToolRequest;

use super::turn_abort::TurnAbortCode;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ToolCallSignature {
    tool: String,
    arguments: String,
}

impl ToolCallSignature {
    fn from_request(request: &ToolRequest) -> Option<Self> {
        let call = request.tool_call.as_ref().ok()?;
        let arguments = call.arguments.as_ref().map_or_else(
            || "null".to_string(),
            |args| serde_json::Value::Object(args.clone()).to_string(),
        );
        Some(Self {
            tool: call.name.to_string(),
            arguments,
        })
    }
}

/// Per-user-turn enforcement state for repetition denials.
#[derive(Debug, Default)]
pub struct TurnToolGuard {
    blocked: HashSet<ToolCallSignature>,
}

impl TurnToolGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a denial already decided by `RepetitionInspector`.
    ///
    /// The first valid denial this turn is recorded without terminating so the
    /// model receives one feedback opportunity. Every later valid denial is
    /// recorded and produces the terminal code, regardless of its signature.
    pub fn enforce_denial(&mut self, request: &ToolRequest) -> Option<TurnAbortCode> {
        let signature = ToolCallSignature::from_request(request)?;
        let is_first_denial = self.blocked.is_empty();
        self.blocked.insert(signature.clone());
        if is_first_denial {
            None
        } else {
            Some(TurnAbortCode::ToolLoop {
                tool: signature.tool,
            })
        }
    }

    pub fn is_blocked(&self, request: &ToolRequest) -> bool {
        ToolCallSignature::from_request(request)
            .is_some_and(|signature| self.blocked.contains(&signature))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_utils::ToolResult;
    use rmcp::model::CallToolRequestParams;
    use serde_json::json;

    fn request(id: &str, tool: &str, arguments: serde_json::Value) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: ToolResult::Ok(CallToolRequestParams {
                task: None,
                meta: None,
                name: tool.to_string().into(),
                arguments: arguments.as_object().cloned(),
            }),
            metadata: None,
            tool_meta: None,
        }
    }

    #[test]
    fn first_policy_denial_blocks_the_signature_without_ending_feedback() {
        let mut guard = TurnToolGuard::new();
        let denied = request("1", "fixture__probe", json!({"command": "first"}));

        assert_eq!(guard.enforce_denial(&denied), None);
        assert!(guard.is_blocked(&denied));
    }

    #[test]
    fn second_policy_denial_is_terminal_for_a_different_signature() {
        let mut guard = TurnToolGuard::new();
        let first = request("1", "fixture__probe", json!({"command": "first"}));
        let second = request("2", "fixture__other", json!({"command": "second"}));

        let _ = guard.enforce_denial(&first);

        assert_eq!(
            guard.enforce_denial(&second),
            Some(TurnAbortCode::ToolLoop {
                tool: "fixture__other".to_string(),
            })
        );
        assert!(guard.is_blocked(&first));
        assert!(guard.is_blocked(&second));
    }

    #[test]
    fn blocking_is_scoped_to_the_exact_arguments() {
        let mut guard = TurnToolGuard::new();
        let denied = request("1", "search", json!({"query": "TP53"}));
        let refined = request("2", "search", json!({"query": "BRCA1"}));

        guard.enforce_denial(&denied);

        assert!(guard.is_blocked(&denied));
        assert!(!guard.is_blocked(&refined));
    }

    #[test]
    fn a_new_user_turn_starts_with_no_blocked_signatures() {
        let denied = request("1", "search", json!({"query": "TP53"}));
        let mut prior_turn = TurnToolGuard::new();
        prior_turn.enforce_denial(&denied);

        let next_turn = TurnToolGuard::new();
        assert!(!next_turn.is_blocked(&denied));
    }
}
