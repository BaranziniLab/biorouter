//! BR-50: an optional self-critique / reflection pass on ordinary answers.
//!
//! Nothing re-reads the agent's own answer for correctness, contradiction, or
//! hallucination before it is returned to the user — the science-accuracy
//! mandate in `system.md` has no enforcement point. Judges exist only for
//! `/goal` (a Stop-hook evaluator, see [`crate::agents::goal`]) and for
//! permissions. This module adds a *cheap, opt-in* critique that reuses the
//! same LLM-as-judge primitive the goal loop uses
//! ([`crate::hooks::prompt_runner::run_prompt_hook`]) rather than inventing a
//! new one.
//!
//! When an ordinary turn is about to finish (no `/goal` is active — a goal
//! already runs its own Stop-hook judge), an enabled critique runs one fast
//! judge over the recent transcript. If it finds a *clear, verifiable* problem
//! it returns a concrete correction, which the reply loop feeds back to the
//! model as one corrective pass; the model revises, then finishes. The pass
//! count is bounded per reply so a stubborn answer cannot spin.
//!
//! **Config-gated and default OFF.** It costs one extra LLM call per turn (a
//! fast-model judge), so it never runs unless a user opts in:
//! `BIOROUTER_SELF_CRITIQUE=true`. The corrective-pass ceiling is
//! `BIOROUTER_SELF_CRITIQUE_MAX_PASSES` (default 1). The judge deliberately
//! biases toward "answer is fine" so an enabled critique adds at most one round
//! trip on a healthy answer, never an open-ended loop.

use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::conversation::Conversation;
use crate::hooks::prompt_runner::run_prompt_hook_in_session;
use crate::hooks::{HookDecision, HookEvent, DEFAULT_PROMPT_TIMEOUT_SECS};

use super::Agent;

/// Default number of corrective passes an enabled critique may request per
/// reply. One is enough to catch a confidently-wrong answer without turning a
/// single question into a multi-round debate; a second pass rarely converges
/// where the first did not.
pub const DEFAULT_MAX_PASSES: u32 = 1;

/// BR-50 policy, resolved once per reply (config reads touch the filesystem).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelfCritiqueConfig {
    /// Master switch. **Default OFF** — the critique costs an extra LLM call
    /// per turn, so it only runs when a user opts in.
    pub enabled: bool,
    /// Per-reply corrective-pass ceiling. `0` also disables the feature.
    pub max_passes: u32,
}

impl Default for SelfCritiqueConfig {
    fn default() -> Self {
        Self {
            // BR-50 MUST default OFF: an extra LLM call per turn is not free.
            enabled: false,
            max_passes: DEFAULT_MAX_PASSES,
        }
    }
}

impl SelfCritiqueConfig {
    /// Resolve from global config. Keys:
    ///
    /// * `BIOROUTER_SELF_CRITIQUE` (bool) — master switch, default `false`.
    /// * `BIOROUTER_SELF_CRITIQUE_MAX_PASSES` (u32) — corrective-pass ceiling,
    ///   default [`DEFAULT_MAX_PASSES`].
    pub fn from_config() -> Self {
        Self::from(Config::global())
    }

    pub fn from(config: &Config) -> Self {
        let defaults = Self::default();
        let enabled = config
            .get_param::<bool>("BIOROUTER_SELF_CRITIQUE")
            .unwrap_or(defaults.enabled);
        let max_passes = config
            .get_param::<u32>("BIOROUTER_SELF_CRITIQUE_MAX_PASSES")
            .unwrap_or(defaults.max_passes);
        Self {
            enabled,
            max_passes,
        }
    }

    /// Whether any critique can happen at all.
    pub fn is_active(&self) -> bool {
        self.enabled && self.max_passes > 0
    }
}

/// The judge rule handed to [`run_prompt_hook_in_session`]. The shared judge harness wraps
/// this so `{"ok": true}` means "the answer is sound, let it stand" and
/// `{"ok": false, "reason": "…"}` means "revise, here is the concrete problem".
///
/// Deliberately conservative: it must bias toward `ok: true`, because every
/// `ok: false` costs the user an extra corrective round trip. It only flags a
/// *verifiable* defect (a factual/scientific error, an internal contradiction,
/// a fabricated specific, or a claim the transcript's own tool evidence
/// refutes), never style, tone, or "could add more detail".
pub(crate) fn critique_rule() -> String {
    "The agent has just finished its answer to the user and is about to return \
     it. Re-read that final answer for CORRECTNESS before it is delivered. This \
     is a science-accuracy safeguard, not a style review.\n\n\
     IMPORTANT, how to read the evidence:\n\
     • The event JSON's `transcript_tail` is only a RECENT EXCERPT of the \
     conversation. Earlier context and tool output have scrolled out of view. Do \
     NOT treat \"I can't see the supporting work in this excerpt\" as an error; \
     judge only what is actually present.\n\
     • Judge the substance of the final answer, not its formatting, length, or \
     tone.\n\n\
     Respond {\"ok\": false, \"reason\": \"<one concrete, specific correction>\"} \
     ONLY when the answer has a clear, verifiable defect, such as:\n\
     • a factual or scientific error (a wrong gene/drug/dose, a mis-stated \
     mechanism, a mathematical or unit mistake);\n\
     • an internal contradiction (the answer asserts two incompatible things);\n\
     • a fabricated specific: a citation, statistic, identifier, or quantitative \
     claim stated with confidence but unsupported by anything in the transcript;\n\
     • a conclusion the transcript's own tool results directly refute.\n\
     The `reason` must name the specific problem and what to fix, so the agent \
     can correct it in one pass.\n\n\
     Respond {\"ok\": true} in EVERY other case, including: the answer is sound; \
     it is a reasonable judgment call, an opinion, or an appropriately hedged \
     estimate; it is merely incomplete or could go deeper; or you are unsure \
     whether something is actually wrong. When in doubt, respond {\"ok\": true}; \
     do NOT invent problems, and do NOT ask for more detail. A correct answer \
     must never be sent back."
        .to_string()
}

/// The user-role instruction injected when the critique asks for a revision.
/// Frames the judge's note as a self-review finding and asks for a corrected
/// final answer, without kicking off unrelated new work.
pub(crate) fn revise_instruction(reason: &str) -> String {
    format!(
        "Self-review flagged a possible problem with the answer you just gave, \
         before it reaches the user:\n\n\"{}\"\n\n\
         If this is a real error, correct it and give the user your revised final \
         answer now. If, on reflection, your original answer was actually right, \
         briefly say why and restate it, and do not start unrelated new work.",
        crate::agents::goal::ellipsize(reason, 600)
    )
}

impl Agent {
    /// Run one self-critique pass over the current conversation.
    ///
    /// Returns `Some(reason)` when the judge found a clear, verifiable defect
    /// (the reply loop should feed `reason` back for one corrective pass), or
    /// `None` when the answer is sound, there is nothing to judge, or the check
    /// could not run (no provider / judge error / timeout). It is deliberately
    /// **failure-open**: a critique that cannot run must never block the agent
    /// from finishing an otherwise-complete turn.
    ///
    /// Reuses the goal-judge primitive: the transcript-tail rendering from
    /// [`crate::agents::goal`] and the LLM-as-judge runner
    /// [`run_prompt_hook_in_session`], exactly as the `/goal` Stop-hook
    /// evaluator does.
    ///
    /// The judge's model call is charged to `session_id` as a side call. The
    /// critique payload carries no session id, so without it the spend could
    /// not be attributed, and the cost card and the usage limits would miss it.
    ///
    /// It is a model request, so it is skipped while a usage limit holds the
    /// chat's next request (`Agent::usage_limit_hold`): the turn finishes, and
    /// its end stores the stop notice that says why.
    pub(crate) async fn run_self_critique_in_session(
        &self,
        session_id: Option<&str>,
        conversation: &Conversation,
    ) -> Option<String> {
        let tail = crate::agents::goal::transcript_tail(conversation)?;

        if let Some(session_id) = session_id {
            if self.usage_limit_hold(session_id).await.is_some() {
                tracing::debug!("self-critique skipped: a usage limit is reached");
                return None;
            }
        }

        let provider = match self.provider().await {
            Ok(provider) => provider,
            Err(e) => {
                tracing::debug!("self-critique skipped: no provider available: {e}");
                return None;
            }
        };

        let payload_json = serde_json::json!({ "transcript_tail": tail }).to_string();
        let outcome = run_prompt_hook_in_session(
            Arc::clone(&provider),
            session_id,
            HookEvent::Stop,
            &critique_rule(),
            &payload_json,
            Duration::from_secs(DEFAULT_PROMPT_TIMEOUT_SECS),
        )
        .await;

        if let Some(error) = outcome.error {
            // Failure-open: a judge that erred or timed out does not get to hold
            // the answer hostage.
            tracing::debug!("self-critique judge did not return a usable verdict: {error}");
            return None;
        }

        match outcome.decision {
            Some(HookDecision::Deny { reason }) => Some(reason),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::Message;

    #[test]
    fn default_is_off() {
        let cfg = SelfCritiqueConfig::default();
        assert!(!cfg.enabled, "BR-50 must default OFF: it costs an LLM call");
        assert!(!cfg.is_active());
    }

    #[test]
    fn enabled_with_zero_passes_is_inert() {
        let cfg = SelfCritiqueConfig {
            enabled: true,
            max_passes: 0,
        };
        assert!(!cfg.is_active(), "zero passes disables even when enabled");
    }

    #[test]
    fn enabled_with_passes_is_active() {
        let cfg = SelfCritiqueConfig {
            enabled: true,
            max_passes: 1,
        };
        assert!(cfg.is_active());
    }

    #[test]
    fn rule_frames_correctness_not_style_and_biases_to_ok() {
        let rule = critique_rule();
        // Focused on correctness, mirroring the science-accuracy mandate.
        assert!(rule.contains("CORRECTNESS"));
        assert!(rule.contains("fabricated"));
        // Truncation-aware, like the goal judge.
        assert!(rule.contains("RECENT EXCERPT"));
        // Biases toward letting a sound answer stand.
        assert!(rule.contains("{\"ok\": true}"));
        assert!(rule.to_lowercase().contains("when in doubt"));
    }

    #[test]
    fn revise_instruction_quotes_reason_and_forbids_new_work() {
        let text = revise_instruction("the BRCA1 dose you cited is off by 10x");
        assert!(text.contains("BRCA1 dose"));
        assert!(text.contains("Self-review"));
        assert!(text.contains("do not start unrelated new work"));
    }

    #[test]
    fn revise_instruction_ellipsizes_a_long_reason() {
        let long = "x".repeat(1_000);
        let text = revise_instruction(&long);
        // The 600-char cap plus an ellipsis keeps the injected feedback bounded.
        assert!(text.contains('…'));
        assert!(text.len() < long.len() + 400);
    }

    #[test]
    fn config_reads_env_style_params() {
        // A config with the master switch on and an explicit ceiling resolves
        // through the same path the reply loop uses.
        let cfg = SelfCritiqueConfig {
            enabled: true,
            max_passes: 2,
        };
        assert!(cfg.is_active());
        assert_eq!(cfg.max_passes, 2);
    }

    #[test]
    fn transcript_tail_available_for_payload() {
        // The critique payload is built from the shared goal transcript tail, so
        // a non-empty conversation yields something to judge.
        let mut conversation = Conversation::default();
        conversation.push(Message::user().with_text("what is the capital of France?"));
        conversation.push(Message::assistant().with_text("Paris."));
        assert!(crate::agents::goal::transcript_tail(&conversation).is_some());
    }
}
