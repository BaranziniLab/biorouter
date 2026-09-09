//! Mistake-streak / recoverable-failure handling (BR-66).
//!
//! Biorouter had no notion of "how many things have gone wrong in a row". Each
//! failure was handled where it happened and then forgotten: a failed tool call
//! became a tool result the model was free to ignore, and a non-context provider
//! error ended the turn outright with a "please retry if you think this is
//! transient" string aimed at the *user*. Nothing counted, so nothing escalated,
//! and a recoverable blip cost the user their turn.
//!
//! This module is the counter. It tracks two streaks over a single reply:
//!
//! * **Tool mistakes** — consecutive *failed* tool calls of any kind (any tool,
//!   any error, plus tool calls the model emitted malformed). At
//!   [`MistakeConfig::nudge_at`] it returns a structured **reflect-and-replan**
//!   nudge, injected alongside the other loop-guard advisories; at
//!   [`MistakeConfig::escalate_at`] the nudge escalates to "stop executing this
//!   plan". The counter resets on any successful tool call — a success is
//!   progress, by definition.
//!
//! * **Provider errors** — consecutive non-context provider failures. Below
//!   [`MistakeConfig::provider_error_retries`] the turn *continues* with a hint
//!   in context (Cline's "one more chance"); at the cap it stops with the
//!   conversation preserved, exactly as it does today.
//!
//! ## Why this is not BR-31
//!
//! [`crate::tool_monitor`]'s no-progress detector (BR-31) answers a narrow,
//! high-confidence question: *has this one tool failed the same way N times in a
//! row?* It is per-tool and per-error-signature, and it earns the right to
//! **block** the next call because a byte-identical failure cannot become a
//! success.
//!
//! BR-66 is the general streak: `read_file` fails, then `shell` fails
//! differently, then a `text_editor` call comes back malformed. No single
//! signature repeats, so BR-31 stays silent — yet the agent has plainly lost the
//! thread. This detector never blocks anything (a mixed run of failures is not
//! proof that the next call is doomed); it only makes the model *stop and
//! re-plan* before it burns the rest of the turn.
//!
//! ## What does not count as a mistake
//!
//! Only failures the model can actually learn from. A call the **user declined**
//! and a call a **loop guard refused to run** are policy verdicts, not the
//! model's failures: counting them would nudge the model for a decision it never
//! made, and pile a second warning on top of the one BR-29/30/31 already sent.
//! The reply loop drops guard denials by request id; user declines are dropped
//! here by [`is_user_decline`].
//!
//! Everything is config-gated — `BIOROUTER_MISTAKE_STREAK_DETECTION=false` turns
//! the whole thing off (restoring the old end-the-turn-on-provider-error
//! behaviour), and each threshold has its own key
//! (see [`MistakeConfig::from_config`]).

use crate::agents::tool_errors::ToolErrorKind;
use crate::config::Config;
use crate::providers::errors::ProviderError;
use crate::tool_monitor::ToolOutcome;

/// Consecutive failed tool calls that earn the reflect-and-replan nudge.
///
/// Three is the number Cline and Aider both settled on: enough that a single
/// unlucky call plus a correction does not trip it, few enough that the model is
/// interrupted while the turn is still salvageable.
pub const DEFAULT_MISTAKE_NUDGE_AT: u32 = 3;
/// Consecutive failed tool calls after which the nudge escalates to "stop
/// executing this plan and either change route or hand it back to the user".
pub const DEFAULT_MISTAKE_ESCALATE_AT: u32 = 6;
/// How many times one reply will re-issue a provider call that failed
/// recoverably. One "more chance with a hint" — a second identical failure is
/// evidence the error is not a blip, and each attempt re-bills the whole context.
pub const DEFAULT_PROVIDER_ERROR_RETRIES: u32 = 1;

/// Stand-in tool name for a tool call the model emitted malformed — there is no
/// real tool name to report, because the call never parsed.
pub const MALFORMED_TOOL_NAME: &str = "a malformed tool call";

/// Normalized-signature prefix of [`crate::agents::tool_execution::DECLINED_RESPONSE`].
/// A declined call comes back flagged `is_error`, so without this it would read
/// as a model mistake. Guarded by a test against the real constant.
const DECLINE_SIGNATURE_PREFIX: &str = "the user has declined to run this tool";

/// BR-66 thresholds. Each stage is independently disablable (`None` / `0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MistakeConfig {
    /// Master switch. When off, the tracker is inert and a provider error ends
    /// the turn exactly as it did before BR-66.
    pub enabled: bool,
    /// Nth consecutive failed tool call that earns a reflect-and-replan nudge,
    /// and the interval at which the nudge repeats. `None` disables the nudge.
    pub nudge_at: Option<u32>,
    /// Nth consecutive failed tool call at which the nudge escalates. `None`
    /// keeps every nudge at the first, hedged level.
    pub escalate_at: Option<u32>,
    /// Recoverable provider errors a single reply will retry before stopping.
    /// `0` restores the end-the-turn-on-first-error behaviour.
    pub provider_error_retries: u32,
}

impl Default for MistakeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            nudge_at: Some(DEFAULT_MISTAKE_NUDGE_AT),
            escalate_at: Some(DEFAULT_MISTAKE_ESCALATE_AT),
            provider_error_retries: DEFAULT_PROVIDER_ERROR_RETRIES,
        }
    }
}

impl MistakeConfig {
    /// Inert: no nudges, and a provider error ends the turn on the first failure.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            nudge_at: None,
            escalate_at: None,
            provider_error_retries: 0,
        }
    }

    /// Resolve from config. Keys:
    ///
    /// * `BIOROUTER_MISTAKE_STREAK_DETECTION` (bool) — master switch.
    /// * `BIOROUTER_MISTAKE_STREAK_NUDGE` (u32, `0` = off) — nudge threshold.
    /// * `BIOROUTER_MISTAKE_STREAK_ESCALATE` (u32, `0` = off) — escalation.
    /// * `BIOROUTER_PROVIDER_ERROR_RETRIES` (u32, `0` = off) — recoverable
    ///   provider-error retries per reply.
    pub fn from_config(config: &Config) -> Self {
        let defaults = Self::default();
        let positive = |value: u32| (value > 0).then_some(value);
        Self {
            enabled: config
                .get_param::<bool>("BIOROUTER_MISTAKE_STREAK_DETECTION")
                .unwrap_or(defaults.enabled),
            nudge_at: config
                .get_param::<u32>("BIOROUTER_MISTAKE_STREAK_NUDGE")
                .ok()
                .map_or(defaults.nudge_at, positive),
            escalate_at: config
                .get_param::<u32>("BIOROUTER_MISTAKE_STREAK_ESCALATE")
                .ok()
                .map_or(defaults.escalate_at, positive),
            provider_error_retries: config
                .get_param::<u32>("BIOROUTER_PROVIDER_ERROR_RETRIES")
                .unwrap_or(defaults.provider_error_retries),
        }
    }
}

/// What the reply loop should do about a provider error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderErrorAction {
    /// The error is recoverable and the retry budget is not spent: put `notice`
    /// in front of the model and take another swing at the turn.
    Recover {
        notice: String,
        attempt: u32,
        limit: u32,
    },
    /// Fatal, or the retry budget is spent: end the turn with `notice`. The
    /// conversation is left intact, so the user can simply say "continue".
    Stop { notice: String },
}

/// Is this a provider error that a second attempt could plausibly survive?
///
/// Deliberately conservative. An auth failure or an unsupported operation will
/// fail identically forever, and a rate limit has *already* exhausted the
/// provider's own backoff — re-issuing any of them just burns a round-trip and
/// buries the real reason under a retry. Transport blips, 5xx, and failed
/// requests are the ones worth one more shot.
///
/// [`ProviderError::ContextLengthExceeded`] never reaches here: the reply loop
/// has its own progressive-compaction ladder for it (BR-13).
pub fn is_recoverable(error: &ProviderError) -> bool {
    match error {
        ProviderError::ServerError(_) => true,
        ProviderError::RequestFailed(_) => matches!(
            error.kind(),
            crate::providers::errors::ProviderErrorKind::Server
                | crate::providers::errors::ProviderErrorKind::Network
        ),
        ProviderError::ExecutionError(_) | ProviderError::UsageError(_) => matches!(
            error.kind(),
            crate::providers::errors::ProviderErrorKind::Server
                | crate::providers::errors::ProviderErrorKind::Network
                | crate::providers::errors::ProviderErrorKind::Other
        ),
        ProviderError::Authentication(_)
        | ProviderError::RateLimitExceeded { .. }
        | ProviderError::NotImplemented(_)
        | ProviderError::ContextLengthExceeded(_) => false,
    }
}

/// The mistake counter for one reply. Lives as a reply-loop local (like the
/// stall tracker), so it starts clean on every user turn and can never leak a
/// streak across sessions.
#[derive(Debug, Default, Clone)]
pub struct MistakeTracker {
    /// Consecutive failed tool calls, across iterations of this reply.
    streak: u32,
    /// The streak length the last nudge was issued at, so one run of failures
    /// nudges once per `nudge_at` failures rather than on every single one.
    nudged_at: u32,
    /// Distinct tools in the current streak, in first-seen order.
    tools: Vec<String>,
    /// BR-51: distinct failure classes in the current streak, in first-seen order.
    /// A run of `timeout`s means something very different from a run of
    /// `invalid_args`, and the nudge now says which it is looking at.
    kinds: Vec<ToolErrorKind>,
    /// Recoverable provider errors already retried in this reply.
    provider_errors: u32,
}

impl MistakeTracker {
    /// Consecutive failed tool calls so far.
    pub fn streak(&self) -> u32 {
        self.streak
    }

    /// Recoverable provider errors already retried in this reply.
    pub fn provider_errors(&self) -> u32 {
        self.provider_errors
    }

    /// Fold in the outcomes of the batch of tool calls that just finished, in
    /// request order, and return the nudge (if any) owed to the model.
    ///
    /// Called at the result-collection seam, so the nudge lands with the failing
    /// results still in front of the model rather than one wasted provider
    /// round-trip later.
    pub fn observe_tool_outcomes(
        &mut self,
        config: &MistakeConfig,
        outcomes: &[ToolOutcome],
    ) -> Option<String> {
        if !config.enabled {
            return None;
        }

        for outcome in outcomes {
            if outcome.failure.is_some() {
                self.streak = self.streak.saturating_add(1);
                if !self.tools.iter().any(|tool| tool == &outcome.tool_name) {
                    self.tools.push(outcome.tool_name.clone());
                }
                if let Some(kind) = outcome.kind {
                    if !self.kinds.contains(&kind) {
                        self.kinds.push(kind);
                    }
                }
            } else {
                // A tool call worked. Whatever the model was doing wrong, it is
                // making progress again.
                self.reset_tool_streak();
            }
        }

        let nudge_at = config.nudge_at.filter(|at| *at > 0)?;
        if self.streak < nudge_at || self.streak < self.nudged_at.saturating_add(nudge_at) {
            return None;
        }
        self.nudged_at = self.streak;

        let escalated = config
            .escalate_at
            .is_some_and(|at| at > 0 && self.streak >= at);
        Some(reflect_nudge(
            self.streak,
            &self.tools,
            &self.kinds,
            escalated,
        ))
    }

    /// BR-51: is every failure in the current streak one a retry could survive
    /// (timeouts, transient dependency errors)? An all-retryable streak says the
    /// *environment* is flaky, not that the model's plan is wrong — a distinction
    /// the streak counter could not make before the taxonomy existed.
    ///
    /// `false` for an empty streak and for any streak with an unclassified
    /// failure in it: the conservative answer is "this is a real mistake".
    pub fn streak_is_all_retryable(&self) -> bool {
        self.streak > 0
            && !self.kinds.is_empty()
            && self.kinds.iter().copied().all(ToolErrorKind::retryable)
    }

    /// A provider response arrived. Whatever went wrong before, the provider is
    /// answering again — the error streak is over.
    pub fn observe_provider_success(&mut self) {
        self.provider_errors = 0;
    }

    /// A provider call failed. Decide whether this reply gets another attempt.
    pub fn observe_provider_error(
        &mut self,
        config: &MistakeConfig,
        error: &ProviderError,
    ) -> ProviderErrorAction {
        let spent = self.provider_errors;
        if !config.enabled || !is_recoverable(error) || spent >= config.provider_error_retries {
            return ProviderErrorAction::Stop {
                notice: stop_notice(error, spent),
            };
        }

        self.provider_errors = spent.saturating_add(1);
        ProviderErrorAction::Recover {
            notice: recovery_notice(error, self.provider_errors, config.provider_error_retries),
            attempt: self.provider_errors,
            limit: config.provider_error_retries,
        }
    }

    fn reset_tool_streak(&mut self) {
        self.streak = 0;
        self.nudged_at = 0;
        self.tools.clear();
        self.kinds.clear();
    }
}

/// Is this outcome a call the *user* declined rather than a failure of the
/// model's? A decline already tells the model, in the tool result itself, to
/// stop and explain — counting it as a mistake would nudge the model for the
/// user's decision.
pub fn is_user_decline(outcome: &ToolOutcome) -> bool {
    outcome
        .failure
        .as_deref()
        .is_some_and(|signature| signature.starts_with(DECLINE_SIGNATURE_PREFIX))
}

/// The structured reflect-and-replan nudge. It does not tell the model *what* to
/// do (Biorouter does not know); it forces it to say what it learned and what it
/// will change, which is the step a stuck agent skips.
///
/// BR-51: it now also names the *classes* of failure in the streak. "Your plan is
/// wrong" is the right message for a run of `not_found` / `invalid_args`, and the
/// wrong one for a run of `timeout` — where the plan may be fine and the world is
/// merely flaky.
fn reflect_nudge(
    streak: u32,
    tools: &[String],
    kinds: &[ToolErrorKind],
    escalated: bool,
) -> String {
    let which = match tools {
        [] => String::new(),
        [one] => format!(" (all of them '{one}')"),
        many => format!(" (across {})", quoted_list(many)),
    };
    let classes = failure_classes(kinds);

    if escalated {
        format!(
            "Reflect and replan, second warning: {streak} tool calls in a row have now \
             failed{which}, and the earlier warning changed nothing. Your current plan is \
             not working. Stop executing it. Either take a materially different route, or \
             stop now and tell the user exactly what is blocking you and what you need \
             from them. Do not keep trying variations of what has already failed.{classes}"
        )
    } else {
        format!(
            "Reflect and replan: your last {streak} tool calls all failed{which}. A run of \
             failures usually means an assumption behind the plan is wrong, not that the \
             next call needs a small tweak. Before calling another tool, say briefly: what \
             the failures actually told you, which assumption of yours they contradict, and \
             what you will do differently: a different tool, a different input, or stopping \
             to ask the user for what you are missing.{classes}"
        )
    }
}

/// The taxonomy clause appended to the reflect nudge. Empty when nothing in the
/// streak was classified, so a taxonomy-less streak reads exactly as it did
/// before BR-51.
fn failure_classes(kinds: &[ToolErrorKind]) -> String {
    if kinds.is_empty() {
        return String::new();
    }
    let named = kinds
        .iter()
        .map(|kind| format!("'{}'", kind.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    if kinds.iter().copied().all(ToolErrorKind::retryable) {
        format!(
            " (These failures were all classed {named}, i.e. retryable, so the environment may \
             be at fault rather than your plan. A bounded retry or a different route is \
             reasonable; hammering the same call is not.)"
        )
    } else {
        format!(
            " (These failures were classed {named}. The ones that are not retryable will \
             fail identically until you change something.)"
        )
    }
}

fn quoted_list(tools: &[String]) -> String {
    tools
        .iter()
        .map(|tool| format!("'{tool}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Model-visible hint injected before the retry. The model cannot fix a 502, but
/// it *can* stop re-sending the request that provoked a 400, and it should know
/// the previous attempt never landed.
fn recovery_notice(error: &ProviderError, attempt: u32, limit: u32) -> String {
    format!(
        "The previous model call did not complete: {error}. Nothing you did in that step \
         took effect, and no tool ran. Biorouter is retrying it ({attempt}/{limit}). If the \
         failure looks like something about the request itself (an oversized or malformed \
         payload, an unsupported argument), send a smaller or simpler step this time rather \
         than repeating the same one; otherwise just carry on where you left off."
    )
}

/// The user-facing message when the turn ends on a provider error. The first
/// sentence is unchanged from before BR-66 — only the retry count is new, so the
/// user is not told to "retry" a call Biorouter already silently retried.
fn stop_notice(error: &ProviderError, retried: u32) -> String {
    let retried_clause = match retried {
        0 => String::new(),
        1 => " Biorouter already retried it once.".to_string(),
        n => format!(" Biorouter already retried it {n} times."),
    };
    format!(
        "Ran into this error: {}\n\nPlease retry if you think this is a transient or \
         recoverable error.{retried_clause}",
        end_sentence(&error.to_string())
    )
}

/// One full stop, not two.
///
/// Vendor error text usually ends in punctuation of its own, and appending a
/// period to it produced `…then try again..` in the chat — the sort of detail
/// that makes a real message look machine-assembled. The frame still supplies
/// the stop when the text has none, which is the common case for a bare
/// `Server error: 502`.
///
/// Deliberately conservative: only `.`, `!`, `?` and a closing quote or bracket
/// after one of them count as an ending. Anything else gets the period it needs.
fn end_sentence(text: &str) -> String {
    let trimmed = text.trim_end();
    let ends = trimmed
        .chars()
        .rev()
        .find(|c| !matches!(c, '"' | '\'' | ')' | ']' | '}' | '\u{201d}' | '\u{2019}'))
        .is_some_and(|c| matches!(c, '.' | '!' | '?'));
    if trimmed.is_empty() || ends {
        trimmed.to_string()
    } else {
        format!("{trimmed}.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::tool_execution::DECLINED_RESPONSE;
    use crate::tool_monitor::tool_outcome;
    use rmcp::model::{CallToolResult, Content};

    fn failed(tool: &str) -> ToolOutcome {
        ToolOutcome {
            tool_name: tool.to_string(),
            failure: Some(format!("{tool} blew up")),
            kind: Some(ToolErrorKind::ToolFailure),
        }
    }

    /// A failure of a class a retry could survive (BR-51).
    fn failed_transiently(tool: &str) -> ToolOutcome {
        ToolOutcome {
            tool_name: tool.to_string(),
            failure: Some(format!("{tool} could not reach the server")),
            kind: Some(ToolErrorKind::Transient),
        }
    }

    fn succeeded(tool: &str) -> ToolOutcome {
        ToolOutcome {
            tool_name: tool.to_string(),
            failure: None,
            kind: None,
        }
    }

    #[test]
    fn a_short_run_of_failures_is_not_a_streak() {
        let config = MistakeConfig::default();
        let mut tracker = MistakeTracker::default();

        assert!(tracker
            .observe_tool_outcomes(&config, &[failed("shell")])
            .is_none());
        assert!(tracker
            .observe_tool_outcomes(&config, &[failed("read_file")])
            .is_none());
        assert_eq!(tracker.streak(), 2);
    }

    #[test]
    fn three_mixed_failures_in_a_row_earn_a_reflect_nudge() {
        let config = MistakeConfig::default();
        let mut tracker = MistakeTracker::default();

        tracker.observe_tool_outcomes(&config, &[failed("shell")]);
        tracker.observe_tool_outcomes(&config, &[failed("read_file")]);
        let nudge = tracker
            .observe_tool_outcomes(&config, &[failed("text_editor")])
            .expect("three consecutive failures is a streak");

        assert!(nudge.contains("Reflect and replan"), "{nudge}");
        assert!(nudge.contains('3'), "the nudge states the streak: {nudge}");
        // Different tools, different errors: BR-31 would never have fired here.
        assert!(nudge.contains("'shell'"), "{nudge}");
        assert!(nudge.contains("'text_editor'"), "{nudge}");
    }

    #[test]
    fn a_whole_failing_batch_counts_call_by_call() {
        let config = MistakeConfig::default();
        let mut tracker = MistakeTracker::default();

        // Three parallel calls, all failing, in one iteration.
        let nudge = tracker.observe_tool_outcomes(
            &config,
            &[failed("shell"), failed("shell"), failed("read_file")],
        );
        assert!(
            nudge.is_some(),
            "a batch of three failures is three mistakes"
        );
        assert_eq!(tracker.streak(), 3);
    }

    #[test]
    fn a_success_resets_the_streak() {
        let config = MistakeConfig::default();
        let mut tracker = MistakeTracker::default();

        tracker.observe_tool_outcomes(&config, &[failed("shell"), failed("shell")]);
        tracker.observe_tool_outcomes(&config, &[succeeded("shell")]);
        assert_eq!(tracker.streak(), 0);

        assert!(
            tracker
                .observe_tool_outcomes(&config, &[failed("shell"), failed("shell")])
                .is_none(),
            "the pre-success failures must not count toward the new streak"
        );
    }

    #[test]
    fn the_nudge_does_not_repeat_on_every_further_failure() {
        let config = MistakeConfig::default();
        let mut tracker = MistakeTracker::default();

        assert!(tracker
            .observe_tool_outcomes(
                &config,
                &[failed("shell"), failed("shell"), failed("shell")]
            )
            .is_some());
        assert!(
            tracker
                .observe_tool_outcomes(&config, &[failed("shell")])
                .is_none(),
            "4th failure: already nudged"
        );
        assert!(
            tracker
                .observe_tool_outcomes(&config, &[failed("shell")])
                .is_none(),
            "5th failure: still quiet"
        );

        let escalated = tracker
            .observe_tool_outcomes(&config, &[failed("shell")])
            .expect("6th consecutive failure re-fires, escalated");
        assert!(escalated.contains("second warning"), "{escalated}");
        assert!(escalated.contains("Stop executing it"), "{escalated}");
    }

    #[test]
    fn a_malformed_tool_call_is_a_mistake_like_any_other() {
        let config = MistakeConfig::default();
        let mut tracker = MistakeTracker::default();

        let malformed = ToolOutcome {
            tool_name: MALFORMED_TOOL_NAME.to_string(),
            failure: Some("invalid json in arguments".to_string()),
            kind: Some(ToolErrorKind::InvalidArgs),
        };
        let nudge = tracker
            .observe_tool_outcomes(&config, &[failed("shell"), malformed.clone(), malformed]);
        assert!(nudge.is_some(), "the model's own bad calls count");
    }

    #[test]
    fn the_detector_is_inert_when_disabled() {
        let config = MistakeConfig::disabled();
        let mut tracker = MistakeTracker::default();

        assert!(tracker
            .observe_tool_outcomes(
                &config,
                &[
                    failed("shell"),
                    failed("shell"),
                    failed("shell"),
                    failed("shell")
                ]
            )
            .is_none());
        assert_eq!(tracker.streak(), 0, "a disabled tracker counts nothing");
    }

    #[test]
    fn nudge_can_be_disabled_on_its_own() {
        let config = MistakeConfig {
            nudge_at: None,
            ..MistakeConfig::default()
        };
        let mut tracker = MistakeTracker::default();

        assert!(tracker
            .observe_tool_outcomes(
                &config,
                &[failed("shell"), failed("shell"), failed("shell")]
            )
            .is_none());
        assert_eq!(tracker.streak(), 3, "still counted, just never announced");
    }

    #[test]
    fn a_user_decline_is_not_the_models_mistake() {
        // The real constant, through the real normalization the reply loop uses.
        let declined = tool_outcome(
            "shell",
            &Ok(CallToolResult {
                content: vec![Content::text(DECLINED_RESPONSE)],
                structured_content: None,
                is_error: Some(true),
                meta: None,
            }),
        );
        assert!(declined.failure.is_some(), "a decline is flagged is_error");
        assert!(
            is_user_decline(&declined),
            "and must be recognised as a decline, not a mistake"
        );
        assert!(!is_user_decline(&failed("shell")));
        assert!(!is_user_decline(&succeeded("shell")));
    }

    #[test]
    fn a_recoverable_provider_error_gets_exactly_one_more_chance() {
        let config = MistakeConfig::default();
        let mut tracker = MistakeTracker::default();
        let error = ProviderError::ServerError("502 bad gateway".to_string());

        let first = tracker.observe_provider_error(&config, &error);
        let ProviderErrorAction::Recover {
            notice,
            attempt,
            limit,
        } = first
        else {
            panic!("a 502 is recoverable: {first:?}");
        };
        assert_eq!((attempt, limit), (1, 1));
        assert!(notice.contains("did not complete"), "{notice}");
        assert!(notice.contains("no tool ran"), "{notice}");

        let second = tracker.observe_provider_error(&config, &error);
        let ProviderErrorAction::Stop { notice } = second else {
            panic!("the retry budget is spent: {second:?}");
        };
        assert!(notice.contains("Ran into this error"), "{notice}");
        assert!(
            notice.contains("already retried it once"),
            "the user is told Biorouter already tried: {notice}"
        );
    }

    #[test]
    fn a_fatal_provider_error_is_never_retried() {
        let config = MistakeConfig::default();
        let mut tracker = MistakeTracker::default();

        for error in [
            ProviderError::Authentication("bad key".to_string()),
            ProviderError::ExecutionError(
                "Authentication error: status 401 Unauthorized".to_string(),
            ),
            ProviderError::UsageError("provider returned insufficient_quota".to_string()),
            ProviderError::NotImplemented("no tools".to_string()),
            ProviderError::RateLimitExceeded {
                details: "slow down".to_string(),
                retry_delay: None,
            },
        ] {
            let action = tracker.observe_provider_error(&config, &error);
            assert!(
                matches!(action, ProviderErrorAction::Stop { .. }),
                "{error} must not be retried, got {action:?}"
            );
        }
        assert_eq!(tracker.provider_errors(), 0, "no retry budget was spent");
    }

    #[test]
    fn a_successful_response_clears_the_provider_error_streak() {
        let config = MistakeConfig::default();
        let mut tracker = MistakeTracker::default();
        let error = ProviderError::RequestFailed("connection reset".to_string());

        assert!(matches!(
            tracker.observe_provider_error(&config, &error),
            ProviderErrorAction::Recover { .. }
        ));
        tracker.observe_provider_success();
        assert_eq!(tracker.provider_errors(), 0);

        assert!(
            matches!(
                tracker.observe_provider_error(&config, &error),
                ProviderErrorAction::Recover { .. }
            ),
            "a later, unrelated blip gets its own chance"
        );
    }

    #[test]
    fn disabling_retries_restores_the_old_end_the_turn_behaviour() {
        let config = MistakeConfig {
            provider_error_retries: 0,
            ..MistakeConfig::default()
        };
        let mut tracker = MistakeTracker::default();
        let error = ProviderError::ServerError("502".to_string());

        let ProviderErrorAction::Stop { notice } = tracker.observe_provider_error(&config, &error)
        else {
            panic!("retries are off");
        };
        assert_eq!(
            notice,
            "Ran into this error: Server error: 502.\n\nPlease retry if you think this is a \
             transient or recoverable error.",
            "the pre-BR-66 text, byte for byte"
        );
    }

    /// Vendor text already ends in a full stop, so the frame must not add a
    /// second one. Measured in the desktop app on a Claude Code turn against an
    /// unsupported model: `…then try again..`.
    #[test]
    fn vendor_text_that_already_ends_in_a_stop_does_not_get_a_second_one() {
        let config = MistakeConfig {
            provider_error_retries: 0,
            ..MistakeConfig::default()
        };
        let mut tracker = MistakeTracker::default();
        let error = ProviderError::RequestFailed(
            "API Error: 400 Claude Code 2.1.235 does not support this model; version 2.1.251 or \
             newer is required. Run 'claude update', or update the Claude desktop app, then try \
             again."
                .to_string(),
        );

        let ProviderErrorAction::Stop { notice } = tracker.observe_provider_error(&config, &error)
        else {
            panic!("retries are off");
        };
        assert!(
            notice.contains("then try again.\n\n"),
            "one stop, not two: {notice}"
        );
        assert!(!notice.contains(".."), "{notice}");
    }

    #[test]
    fn end_sentence_only_supplies_a_stop_that_is_missing() {
        assert_eq!(end_sentence("Server error: 502"), "Server error: 502.");
        assert_eq!(end_sentence("try again."), "try again.");
        assert_eq!(end_sentence("what now?"), "what now?");
        assert_eq!(end_sentence("stop!"), "stop!");
        // Trailing whitespace is not an ending, and a stop inside a closing
        // quote or bracket still counts as one.
        assert_eq!(end_sentence("done.  "), "done.");
        assert_eq!(end_sentence("(see the log.)"), "(see the log.)");
        assert_eq!(end_sentence("he said \"no.\""), "he said \"no.\"");
        assert_eq!(end_sentence("a bare clause"), "a bare clause.");
        assert_eq!(end_sentence(""), "");
    }

    // ── BR-51: the streak now knows what kind of failures it is counting ──

    /// A run of flaky-environment failures must not be diagnosed as "your plan is
    /// wrong" — the model's plan may be fine and the world merely down.
    #[test]
    fn an_all_retryable_streak_is_named_as_such() {
        let config = MistakeConfig::default();
        let mut tracker = MistakeTracker::default();

        let nudge = tracker
            .observe_tool_outcomes(
                &config,
                &[
                    failed_transiently("fetch"),
                    failed_transiently("fetch"),
                    failed_transiently("fetch"),
                ],
            )
            .expect("three failures nudge");

        assert!(tracker.streak_is_all_retryable());
        assert!(nudge.contains("'transient'"), "{nudge}");
        assert!(nudge.contains("retryable"), "{nudge}");
    }

    /// One hard failure in the run is enough: this is a real mistake streak.
    #[test]
    fn a_mixed_streak_is_not_retryable() {
        let config = MistakeConfig::default();
        let mut tracker = MistakeTracker::default();

        let nudge = tracker
            .observe_tool_outcomes(
                &config,
                &[
                    failed_transiently("fetch"),
                    failed("shell"),
                    failed_transiently("fetch"),
                ],
            )
            .expect("three failures nudge");

        assert!(!tracker.streak_is_all_retryable());
        assert!(nudge.contains("not retryable"), "{nudge}");
    }

    /// A success clears the classes with the streak — the next run of failures
    /// must be diagnosed on its own evidence, not the last one's.
    #[test]
    fn a_success_clears_the_recorded_classes() {
        let config = MistakeConfig::default();
        let mut tracker = MistakeTracker::default();

        tracker.observe_tool_outcomes(&config, &[failed("shell"), succeeded("shell")]);
        assert!(!tracker.streak_is_all_retryable());

        let nudge = tracker
            .observe_tool_outcomes(
                &config,
                &[
                    failed_transiently("fetch"),
                    failed_transiently("fetch"),
                    failed_transiently("fetch"),
                ],
            )
            .expect("three fresh failures nudge");
        assert!(tracker.streak_is_all_retryable());
        assert!(!nudge.contains("'tool_failure'"), "{nudge}");
    }
}
