//! Progress-stall detection for ordinary chat (BR-32).
//!
//! Biorouter already had genuinely good local-minimum detection — fuzzy
//! similarity of an LLM judge's feedback across attempts, a stall counter that
//! does **not** reset when tools run, and a graceful give-up — but it lived
//! entirely inside the `/goal` Stop-hook loop ([`super::goal`]) and therefore
//! never ran for ordinary chat, which is where most stuck loops actually happen.
//!
//! This module generalizes that machinery:
//!
//! * [`reason_similarity`] / [`similar_reasons`] — the Jaccard word-set
//!   similarity that used to be private to `goal.rs`; the goal loop now uses
//!   this copy, so the two can never drift.
//! * [`StallWatch`] — the goal's `stall_count` / `last_reason` bookkeeping,
//!   lifted out of `GoalState` into a reusable tracker with an explicit
//!   [`StallAction`] outcome (nudge → give up), mirroring `GoalOutcome`.
//! * [`check_progress`] — a *periodic* "are you looping?" LLM check over a
//!   compact transcript tail (Gemini CLI's periodic loop check), run from the
//!   reply loop for **every** session, not just goal sessions.
//!
//! ## Why periodic, and why it's cheap
//!
//! An always-on LLM loop check would add a provider round-trip to every
//! iteration of every turn. Instead the check only starts once a single turn has
//! already run [`DEFAULT_STALL_CHECK_AFTER`] actions without returning to the
//! user (30 — normal chat never gets there; a stuck agentic turn does), and then
//! only every [`DEFAULT_STALL_CHECK_EVERY`] actions (10). It runs on the
//! provider's *fast* model when one is configured (like prompt hooks), sees only
//! a bounded tail, and is fail-open: any provider error, timeout or unparseable
//! verdict is treated as "no evidence of a loop".
//!
//! ## Staged response (mirrors BR-29's soft-then-hard shape)
//!
//! 1. First "you're looping" verdict → a **nudge**: a model-visible message
//!    naming the loop and demanding a different approach. Nothing is blocked.
//! 2. The checker keeps flagging the *same* loop ([`StallCheckConfig::stall_limit`]
//!    near-identical reasons in a row) or flags any loop
//!    [`StallCheckConfig::max_flags`] times without intervening progress → **give up**: the model is told to
//!    stop and hand the user a best-effort answer, and the turn is hard-bounded
//!    to a short grace period after that so a model that ignores the instruction
//!    cannot keep spinning after confirmed lack of progress.
//!
//! Everything is config-gated: `BIOROUTER_STALL_CHECK=false` turns the whole
//! thing off, and each threshold has its own key (see [`StallCheckConfig::from_config`]).

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tracing::debug;

use crate::config::Config;
use crate::conversation::message::Message;
use crate::conversation::Conversation;
use crate::providers::base::Provider;

/// Jaccard similarity (over word sets) at/above which two stall reasons are
/// treated as "the same complaint" — i.e. the loop has not changed shape.
pub const STALL_SIMILARITY: f32 = 0.5;

/// Actions in a single turn before the first "are you looping?" check runs.
/// Well past any normal chat turn, so the common case never pays for it.
pub const DEFAULT_STALL_CHECK_AFTER: u32 = 30;
/// Actions between subsequent checks once the first one has run.
pub const DEFAULT_STALL_CHECK_EVERY: u32 = 10;
/// Consecutive near-identical "you're looping" reasons that count as a
/// confirmed, non-converging stall (the generalization of `GOAL_STALL_LIMIT`).
pub const DEFAULT_STALL_LIMIT: u32 = 2;
/// Consecutive "you're looping" verdicts without intervening progress before giving up
/// even if the reasons keep changing (the generalization of `GOAL_MAX_ITERATIONS`).
pub const DEFAULT_STALL_MAX_FLAGS: u32 = 4;
/// Iterations the model gets to deliver its best-effort answer after a give-up
/// before the turn is ended for it.
pub const STALL_WRAPUP_GRACE: u32 = 2;

/// Wall-clock budget for one loop check. Fail-open on timeout.
const STALL_CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// Max messages rendered into the tail handed to the loop checker.
const TAIL_MESSAGES: usize = 24;
/// Per-entry character cap in the tail.
const TAIL_ENTRY_MAX_CHARS: usize = 400;
/// Total character cap of the tail.
const TAIL_MAX_CHARS: usize = 6_000;

/// Truncate to `max` characters on a char boundary, appending an ellipsis.
///
/// Lives here (rather than in `goal.rs`, where it started) because both the goal
/// loop and the general stall detector need it; `goal` re-exports it.
pub(crate) fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{truncated}…")
}

/// Lowercase alphanumeric word set of a string, for similarity comparison.
fn word_set(s: &str) -> std::collections::HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(|w| w.to_ascii_lowercase())
        .collect()
}

/// Jaccard similarity of two stall reasons' word sets, in `[0, 1]`.
pub fn reason_similarity(a: &str, b: &str) -> f32 {
    let (sa, sb) = (word_set(a), word_set(b));
    if sa.is_empty() && sb.is_empty() {
        return 1.0;
    }
    let inter = sa.intersection(&sb).count() as f32;
    let union = sa.union(&sb).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// Whether two stall reasons describe the same non-converging situation.
pub fn similar_reasons(a: &str, b: &str) -> bool {
    reason_similarity(a, b) >= STALL_SIMILARITY
}

/// Configuration for the periodic chat stall check.
#[derive(Debug, Clone)]
pub struct StallCheckConfig {
    /// Master switch (`BIOROUTER_STALL_CHECK`).
    pub enabled: bool,
    /// Actions in a turn before the first check.
    pub first_check_at: u32,
    /// Actions between checks thereafter.
    pub interval: u32,
    /// Consecutive near-identical looping reasons that force a give-up.
    pub stall_limit: u32,
    /// Looping verdicts without intervening progress that force a give-up.
    pub max_flags: u32,
}

impl Default for StallCheckConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            first_check_at: DEFAULT_STALL_CHECK_AFTER,
            interval: DEFAULT_STALL_CHECK_EVERY,
            stall_limit: DEFAULT_STALL_LIMIT,
            max_flags: DEFAULT_STALL_MAX_FLAGS,
        }
    }
}

impl StallCheckConfig {
    /// No periodic loop check at all (the pre-BR-32 behavior).
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }

    /// Resolve from env / `config.yaml`, falling back to the defaults.
    ///
    /// * `BIOROUTER_STALL_CHECK` (bool) — off switch.
    /// * `BIOROUTER_STALL_CHECK_AFTER` (u32) — actions before the first check.
    /// * `BIOROUTER_STALL_CHECK_EVERY` (u32) — actions between checks.
    /// * `BIOROUTER_STALL_LIMIT` (u32) — near-identical flags that end the turn.
    /// * `BIOROUTER_STALL_MAX_FLAGS` (u32) — total flags that end the turn.
    ///
    /// A zero interval is coerced back to the default so a stray `0` cannot turn
    /// the check into a per-iteration provider call.
    pub fn from_config(config: &Config) -> Self {
        let defaults = Self::default();
        let positive = |key: &str, fallback: u32| {
            config
                .get_param::<u32>(key)
                .ok()
                .filter(|value| *value > 0)
                .unwrap_or(fallback)
        };
        Self {
            enabled: config
                .get_param::<bool>("BIOROUTER_STALL_CHECK")
                .unwrap_or(defaults.enabled),
            first_check_at: positive("BIOROUTER_STALL_CHECK_AFTER", defaults.first_check_at),
            interval: positive("BIOROUTER_STALL_CHECK_EVERY", defaults.interval),
            stall_limit: positive("BIOROUTER_STALL_LIMIT", defaults.stall_limit),
            max_flags: positive("BIOROUTER_STALL_MAX_FLAGS", defaults.max_flags),
        }
    }

    /// Is a loop check due at this (1-based) action count of the current turn?
    pub fn due(&self, iteration: u32) -> bool {
        self.enabled
            && iteration >= self.first_check_at
            && (iteration - self.first_check_at).is_multiple_of(self.interval)
    }
}

/// What the reply loop should do after a loop check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StallAction {
    /// No evidence of a loop (or the check was unavailable) — carry on.
    Proceed,
    /// Looping: inject a model-visible nudge naming the loop.
    Nudge { reason: String },
    /// The loop is not converging — stop and deliver a best-effort answer.
    GiveUp {
        reason: String,
        /// How many times the checker flagged a loop this turn.
        flags: u32,
        /// True when the same complaint repeated (vs. the flag ceiling).
        stalled: bool,
    },
}

/// Per-turn stall bookkeeping: the goal loop's `stall_count` / `last_reason`,
/// generalized. Reset by a clean verdict — real progress clears the streak,
/// exactly like a passing goal evaluation.
#[derive(Debug, Default)]
pub struct StallWatch {
    last_reason: Option<String>,
    stall_count: u32,
    flags: u32,
    given_up: bool,
}

impl StallWatch {
    /// The checker's most recent complaint, fed back into the next check so it
    /// can judge whether anything actually changed.
    pub fn last_reason(&self) -> Option<&str> {
        self.last_reason.as_deref()
    }

    /// Has this turn already given up? (Used to stop re-checking a turn that is
    /// already wrapping up.)
    pub fn has_given_up(&self) -> bool {
        self.given_up
    }

    /// Clear earlier suspicions when completed work demonstrates progress.
    pub fn record_progress(&mut self) {
        if !self.given_up {
            self.last_reason = None;
            self.stall_count = 0;
            self.flags = 0;
        }
    }

    /// Fold one verdict into the watch. `None` = "no loop detected / no verdict".
    pub fn record(&mut self, verdict: Option<&str>, config: &StallCheckConfig) -> StallAction {
        let Some(reason) = verdict else {
            self.record_progress();
            return StallAction::Proceed;
        };

        self.flags = self.flags.saturating_add(1);
        let repeated = self
            .last_reason
            .as_deref()
            .map(|prev| similar_reasons(prev, reason))
            .unwrap_or(false);
        self.stall_count = if repeated {
            self.stall_count.saturating_add(1)
        } else {
            0
        };
        self.last_reason = Some(reason.to_string());

        let stalled = self.stall_count >= config.stall_limit;
        let capped = self.flags >= config.max_flags;
        if stalled || capped {
            self.given_up = true;
            StallAction::GiveUp {
                reason: reason.to_string(),
                flags: self.flags,
                stalled,
            }
        } else {
            StallAction::Nudge {
                reason: reason.to_string(),
            }
        }
    }
}

/// A compact rendering of the recent conversation for the loop checker.
///
/// Unlike [`super::goal::transcript_tail`] (which only renders message *text*,
/// because a goal judge reasons about the deliverable), this includes the tool
/// calls and their outcomes — a stuck agentic turn is usually all tool traffic
/// and no prose, so a text-only tail would show the checker nothing to judge.
pub fn progress_tail(conversation: &Conversation) -> Option<String> {
    let mut entries: Vec<String> = Vec::new();
    let mut budget = TAIL_MAX_CHARS;

    for message in conversation.messages().iter().rev() {
        if entries.len() >= TAIL_MESSAGES || budget == 0 {
            break;
        }
        for entry in render_message(message).into_iter().rev() {
            if entries.len() >= TAIL_MESSAGES || budget == 0 {
                break;
            }
            let entry = ellipsize(&entry, TAIL_ENTRY_MAX_CHARS.min(budget));
            budget = budget.saturating_sub(entry.chars().count());
            entries.push(entry);
        }
    }

    if entries.is_empty() {
        return None;
    }
    entries.reverse();
    Some(entries.join("\n"))
}

/// One conversation message → zero or more tail lines (text, tool calls, tool
/// results), in order.
fn render_message(message: &Message) -> Vec<String> {
    let role = match message.role {
        rmcp::model::Role::User => "user",
        rmcp::model::Role::Assistant => "assistant",
    };
    let mut lines = Vec::new();

    for content in &message.content {
        if let Some(request) = content.as_tool_request() {
            match &request.tool_call {
                Ok(call) => {
                    let args = call
                        .arguments
                        .as_ref()
                        .map(|a| serde_json::Value::Object(a.clone()).to_string())
                        .unwrap_or_default();
                    lines.push(format!("tool call: {}({args})", call.name));
                }
                Err(e) => lines.push(format!("tool call: <invalid> ({e})")),
            }
            continue;
        }
        if let Some(response) = content.as_tool_response() {
            let outcome = crate::tool_monitor::tool_outcome("tool", &response.tool_result);
            let body = content.as_tool_response_text().unwrap_or_default();
            let status = if outcome.failure.is_some() {
                "error"
            } else {
                "ok"
            };
            lines.push(format!(
                "tool result [{status}]: {}",
                body.trim().replace('\n', " ")
            ));
            continue;
        }
        // System notifications are UI chrome, not agent work — skip them so they
        // never look like "the agent said something" to the checker.
        if content.as_system_notification().is_some() {
            continue;
        }
        if let Some(text) = content.as_text() {
            let text = text.trim();
            if !text.is_empty() {
                lines.push(format!("{role}: {text}"));
            }
        }
    }

    lines
}

const LOOP_CHECK_SYSTEM_PROMPT: &str = r#"You are a loop monitor for the Biorouter agent. The agent has been working on one user request for many consecutive actions without returning control to the user. You are shown a RECENT EXCERPT of that work (tool calls, tool results, and the agent's own messages).

Decide whether the agent is still making real progress, or is stuck: repeating the same call or the same failure, oscillating between two actions, re-reading things it already read, restating the same plan, or otherwise not advancing toward the user's request.

Respond with ONLY a JSON object, no prose, no code fences:
{"looping": false} if the agent is making progress, or
{"looping": true, "reason": "<one sentence: what it keeps repeating, and what it should do instead>"} if it is stuck.

Be conservative. Waiting for an identified active job, checking changing CI/build results, or reading successive pages is legitimate progress. Repeated tool arguments alone do not prove a loop; compare the results. Long, varied, advancing work is NOT a loop: different tools, different files, new information, or steady movement through a plan all mean {"looping": false}. The excerpt is only a tail, so earlier progress is not visible; never infer a loop from what is missing. Report a loop ONLY when the excerpt itself shows concrete repetition or a lack of progress."#;

#[derive(Deserialize)]
struct LoopVerdict {
    #[serde(default)]
    looping: bool,
    #[serde(default)]
    reason: Option<String>,
}

/// Extract the first JSON object from model output, tolerating code fences and
/// surrounding prose (same tolerance as the prompt-hook judge).
fn parse_verdict(text: &str) -> Option<LoopVerdict> {
    let trimmed = text.trim();
    if let Ok(verdict) = serde_json::from_str::<LoopVerdict>(trimmed) {
        return Some(verdict);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    let candidate = trimmed.get(start..=end)?;
    serde_json::from_str::<LoopVerdict>(candidate).ok()
}

/// Ask the model whether the agent is looping. `Some(reason)` = looping.
///
/// Fail-open by construction: a provider error, a timeout, an unparseable
/// verdict, or a `{"looping": true}` with no reason all return `None`
/// (`{"looping": true}` without a reason is unusable as a nudge, and a loop the
/// checker cannot describe is not evidence enough to end a user's turn).
pub async fn check_progress(
    provider: Arc<dyn Provider>,
    tail: &str,
    actions_taken: u32,
    previous_reason: Option<&str>,
) -> Option<String> {
    let history = match previous_reason {
        Some(prev) => format!(
            "\n\nOn an earlier check this turn you already flagged a loop:\n\"{}\"\n\
             If the agent has NOT moved past that, say so again in the same terms. If it \
             has genuinely changed approach and is advancing, respond {{\"looping\": false}}.",
            ellipsize(prev, 400)
        ),
        None => String::new(),
    };
    let prompt = format!(
        "The agent has taken {actions_taken} actions on this request without returning to \
         the user.{history}\n\nRecent excerpt:\n{tail}"
    );

    let completion = tokio::time::timeout(
        STALL_CHECK_TIMEOUT,
        provider.complete_fast(
            LOOP_CHECK_SYSTEM_PROMPT,
            &[Message::user().with_text(prompt)],
            &[],
        ),
    )
    .await;

    let response = match completion {
        Ok(Ok((message, _usage))) => message.as_concat_text(),
        Ok(Err(e)) => {
            debug!("stall check: provider error, assuming progress: {e}");
            return None;
        }
        Err(_) => {
            debug!("stall check: timed out after {STALL_CHECK_TIMEOUT:?}, assuming progress");
            return None;
        }
    };

    match parse_verdict(&response) {
        Some(LoopVerdict {
            looping: true,
            reason: Some(reason),
        }) if !reason.trim().is_empty() => Some(reason.trim().to_string()),
        Some(_) => None,
        None => {
            debug!(
                "stall check: unparseable verdict, assuming progress: {}",
                response.chars().take(200).collect::<String>()
            );
            None
        }
    }
}

/// Model-visible nudge for the first (and any non-terminal) looping verdict.
pub fn nudge_instruction(reason: &str, actions_taken: u32) -> String {
    format!(
        "Progress check (automatic, {actions_taken} actions into this turn): you appear to be \
         stuck in a loop.\n\nWhat the check saw: {}\n\nDo NOT repeat that action again. Either \
         (a) take a genuinely different approach, (b) state what you have established so far \
         and what is blocking you, or (c) stop and ask the user. If you believe you ARE making \
         progress, say briefly what changed since the last step and continue.",
        ellipsize(reason, 400)
    )
}

/// The instruction fed to the model when the stall check gives up: deliver a
/// best-effort answer instead of starting another pass. Deliberately the same
/// shape as the `/goal` give-up (`goal::giveup_instruction`) — the situation is
/// identical, only the trigger differs.
pub fn giveup_instruction(reason: &str) -> String {
    format!(
        "An automatic progress check has determined that this turn is no longer making \
         progress. Do NOT start another pass or call more tools to redo the work. Give the \
         user your best final answer now, in this turn:\n\
         1. A concise summary of what you DID accomplish.\n\
         2. What remains incomplete.\n\
         3. The specific blocker, and a concrete suggestion for how the user could unblock \
         it (a narrower request, a missing credential, a different tool).\n\
         The progress check's finding was: \"{}\"",
        ellipsize(reason, 400)
    )
}

/// The user-facing message when the model keeps working through the give-up and
/// the turn has to be ended for it.
pub fn stopped_message(reason: &str) -> String {
    format!(
        "I've stopped this turn because I wasn't making progress. I'm not finishing because \
         the task is necessarily complete. The progress check found: {}\n\nTell me how you'd \
         like to proceed (a narrower step, a different approach, or more context). \
         (Disable this check with `BIOROUTER_STALL_CHECK=false`.)",
        ellipsize(reason, 300)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelConfig;
    use crate::providers::base::{ProviderMetadata, ProviderUsage, Usage};
    use crate::providers::errors::ProviderError;
    use async_trait::async_trait;
    use rmcp::model::{CallToolResult, Content, Tool};

    // ── similarity (the goal.rs primitive, now shared) ──

    #[test]
    fn same_complaint_is_similar() {
        let a = "You keep running the same failing grep for `foo` in src/";
        let b = "You are running the same failing grep for `foo` in src/ again";
        assert!(similar_reasons(a, b));
    }

    #[test]
    fn different_complaints_are_not_similar() {
        let a = "You keep re-reading main.rs without changing it";
        let b = "The database credentials are missing, ask the user";
        assert!(!similar_reasons(a, b));
    }

    // ── schedule ──

    #[test]
    fn check_is_not_due_before_the_first_threshold() {
        let config = StallCheckConfig::default();
        for iteration in [1, 5, 29] {
            assert!(
                !config.due(iteration),
                "iteration {iteration} should be early"
            );
        }
    }

    #[test]
    fn check_is_due_at_the_threshold_then_every_interval() {
        let config = StallCheckConfig::default();
        assert!(config.due(30));
        assert!(!config.due(31));
        assert!(!config.due(39));
        assert!(config.due(40));
        assert!(config.due(50));
    }

    #[test]
    fn disabled_config_is_never_due() {
        let config = StallCheckConfig::disabled();
        for iteration in [1, 30, 40, 100] {
            assert!(!config.due(iteration));
        }
    }

    // ── staged escalation ──

    #[test]
    fn first_flag_nudges_and_a_clean_verdict_resets() {
        let config = StallCheckConfig::default();
        let mut watch = StallWatch::default();

        assert_eq!(
            watch.record(Some("re-reading main.rs over and over"), &config),
            StallAction::Nudge {
                reason: "re-reading main.rs over and over".to_string()
            }
        );
        assert_eq!(
            watch.last_reason(),
            Some("re-reading main.rs over and over")
        );

        assert_eq!(watch.record(None, &config), StallAction::Proceed);
        assert_eq!(watch.last_reason(), None);
        assert!(!watch.has_given_up());
    }

    #[test]
    fn repeated_same_complaint_gives_up_as_stalled() {
        let config = StallCheckConfig::default();
        let mut watch = StallWatch::default();

        let reason = "You keep running the same failing cargo build with no change";
        assert!(matches!(
            watch.record(Some(reason), &config),
            StallAction::Nudge { .. }
        ));
        assert!(matches!(
            watch.record(
                Some("You are still running the same failing cargo build"),
                &config
            ),
            StallAction::Nudge { .. }
        ));
        match watch.record(
            Some("Still running that same failing cargo build, unchanged"),
            &config,
        ) {
            StallAction::GiveUp { flags, stalled, .. } => {
                assert_eq!(flags, 3);
                assert!(stalled, "three near-identical complaints is a stall");
            }
            other => panic!("expected give-up, got {other:?}"),
        }
        assert!(watch.has_given_up());
    }

    #[test]
    fn ever_changing_complaints_still_hit_the_flag_ceiling() {
        let config = StallCheckConfig::default();
        let mut watch = StallWatch::default();

        let reasons = [
            "you keep re-reading the same file",
            "now you retry a network fetch that always times out",
            "you are rewriting the identical patch again",
            "you loop between listing and searching without acting",
        ];
        for reason in &reasons[..3] {
            assert!(matches!(
                watch.record(Some(reason), &config),
                StallAction::Nudge { .. }
            ));
        }
        match watch.record(Some(reasons[3]), &config) {
            StallAction::GiveUp { flags, stalled, .. } => {
                assert_eq!(flags, 4);
                assert!(!stalled, "the reasons differed; this is the flag ceiling");
            }
            other => panic!("expected give-up, got {other:?}"),
        }
    }

    #[test]
    fn a_clean_verdict_between_flags_clears_the_stall_streak() {
        let config = StallCheckConfig::default();
        let mut watch = StallWatch::default();
        let reason = "you keep calling the same failing tool";

        watch.record(Some(reason), &config);
        watch.record(None, &config);
        // Same complaint again, but the streak was broken: back to a nudge, not
        // a give-up: a clean verdict also clears the previous flags.
        assert!(matches!(
            watch.record(Some(reason), &config),
            StallAction::Nudge { .. }
        ));
    }

    #[test]
    fn healthy_progress_prevents_historical_flags_from_accumulating() {
        let config = StallCheckConfig::default();
        for explicit_progress in [false, true] {
            let mut watch = StallWatch::default();
            for index in 0..100 {
                assert!(matches!(
                    watch.record(Some(&format!("repeated step {index}")), &config),
                    StallAction::Nudge { .. }
                ));
                if explicit_progress {
                    watch.record_progress();
                } else {
                    assert_eq!(watch.record(None, &config), StallAction::Proceed);
                }
            }
            assert!(!watch.has_given_up());
            assert_eq!(watch.last_reason(), None);
        }
    }

    #[test]
    fn progress_does_not_resurrect_a_turn_already_wrapping_up() {
        let mut watch = StallWatch::default();
        let config = StallCheckConfig::default();
        for _ in 0..3 {
            watch.record(Some("identical unchanged circular reasoning"), &config);
        }
        assert!(watch.has_given_up());
        watch.record_progress();
        assert!(watch.has_given_up());
    }

    // ── verdict parsing ──

    #[test]
    fn parses_plain_and_fenced_verdicts() {
        assert!(!parse_verdict(r#"{"looping": false}"#).unwrap().looping);
        let fenced =
            parse_verdict("```json\n{\"looping\": true, \"reason\": \"same grep\"}\n```").unwrap();
        assert!(fenced.looping);
        assert_eq!(fenced.reason.as_deref(), Some("same grep"));
        assert!(parse_verdict("I think it's fine, honestly").is_none());
    }

    // ── the transcript tail ──

    #[test]
    fn tail_renders_tool_calls_and_outcomes() {
        let mut conversation = Conversation::default();
        conversation.push(Message::user().with_text("fix the build"));
        conversation.push(Message::assistant().with_tool_request(
            "1",
            Ok(rmcp::model::CallToolRequestParams {
                task: None,
                meta: None,
                name: "developer__shell".into(),
                arguments: Some(rmcp::object!({"command": "cargo build"})),
            }),
        ));
        conversation.push(Message::user().with_tool_response(
            "1",
            Ok(CallToolResult {
                content: vec![Content::text("error: cannot find crate `foo`")],
                structured_content: None,
                is_error: Some(true),
                meta: None,
            }),
        ));

        let tail = progress_tail(&conversation).expect("tail");
        assert!(tail.contains("user: fix the build"), "{tail}");
        assert!(tail.contains("tool call: developer__shell"), "{tail}");
        assert!(tail.contains("cargo build"), "{tail}");
        assert!(tail.contains("tool result [error]"), "{tail}");
        assert!(tail.contains("cannot find crate"), "{tail}");
    }

    #[test]
    fn empty_conversation_has_no_tail() {
        assert!(progress_tail(&Conversation::default()).is_none());
    }

    #[test]
    fn tail_is_bounded() {
        let mut conversation = Conversation::default();
        for i in 0..200 {
            conversation.push(Message::assistant().with_text(format!("{i} {}", "x".repeat(2000))));
        }
        let tail = progress_tail(&conversation).expect("tail");
        assert!(
            tail.chars().count() <= TAIL_MAX_CHARS + TAIL_ENTRY_MAX_CHARS,
            "tail was {} chars",
            tail.chars().count()
        );
        // Keeps the *end* of the conversation (the recent work), not the start.
        assert!(
            tail.contains("199"),
            "tail should end at the newest message"
        );
    }

    // ── the LLM check (fail-open) ──

    struct StubProvider {
        response: Result<String, ()>,
    }

    #[async_trait]
    impl Provider for StubProvider {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::new(
                "stub",
                "Stub",
                "",
                "stub-model",
                vec!["stub-model"],
                "",
                vec![],
            )
        }

        fn get_name(&self) -> &str {
            "stub"
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            match &self.response {
                Ok(text) => Ok((
                    Message::assistant().with_text(text),
                    ProviderUsage::new("stub-model".to_string(), Usage::default()),
                )),
                Err(()) => Err(ProviderError::ExecutionError("boom".to_string())),
            }
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail("stub-model")
        }
    }

    fn stub(response: Result<&str, ()>) -> Arc<dyn Provider> {
        Arc::new(StubProvider {
            response: response.map(|s| s.to_string()),
        })
    }

    #[tokio::test]
    async fn looping_verdict_yields_a_reason() {
        let reason = check_progress(
            stub(Ok(
                r#"{"looping": true, "reason": "same failing grep, 6 times"}"#,
            )),
            "tool call: grep(foo)",
            30,
            None,
        )
        .await;
        assert_eq!(reason.as_deref(), Some("same failing grep, 6 times"));
    }

    #[tokio::test]
    async fn clean_verdict_and_failures_are_fail_open() {
        assert!(
            check_progress(stub(Ok(r#"{"looping": false}"#)), "work", 30, None)
                .await
                .is_none()
        );
        // No reason to show the model — not actionable, so not a loop.
        assert!(
            check_progress(stub(Ok(r#"{"looping": true}"#)), "work", 30, None)
                .await
                .is_none()
        );
        assert!(check_progress(stub(Ok("dunno")), "work", 30, None)
            .await
            .is_none());
        assert!(check_progress(stub(Err(())), "work", 30, None)
            .await
            .is_none());
    }
}
