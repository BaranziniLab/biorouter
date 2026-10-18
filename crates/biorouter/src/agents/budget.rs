//! Per-reply wall-clock / token / dollar budget (BR-35).
//!
//! Before this, the only thing that bounded a single reply was the *iteration*
//! count (`max_turns`, and since BR-34 `max_tool_calls`). Neither is a bound on
//! time or money: provider 429 backoff can add ~2 minutes **inside** one
//! iteration, and one iteration can pull a 200k-token context. A throttled or
//! pathological session could therefore run far longer — and cost far more —
//! than the user expected, with nothing to say so.
//!
//! This module adds the missing axes:
//!
//! * **wall clock** — seconds since the reply started;
//! * **tokens** — the provider-reported token count of every turn in this reply,
//!   summed. Each turn re-bills its whole input context, so this sum is what the
//!   provider actually charged for, not the size of the conversation;
//! * **dollars** — the same tokens priced through [`crate::providers::pricing`]
//!   (provider overrides first, then the canonical catalog). Unknown pricing
//!   simply means the dollar axis never fires; the other two still do.
//!
//! ## Off unless asked for
//!
//! Every limit is `None` by default, so a stock Biorouter behaves exactly as it
//! did. A limit can be set per session ([`crate::agents::types::SessionConfig::budget`])
//! or globally via config/env (`BIOROUTER_REPLY_BUDGET_SECONDS`,
//! `BIOROUTER_REPLY_BUDGET_TOKENS`, `BIOROUTER_REPLY_BUDGET_USD`); the session
//! value wins per-axis. See [`ReplyBudget::resolve`].
//!
//! ## Graceful, not a kill (staged like BR-29/BR-32)
//!
//! 1. At [`BUDGET_WARN_FRACTION`] of any limit the user gets **one** heads-up
//!    notification — the progress meter, so a long turn is never a silent spend.
//! 2. On exceeding a limit the model is told, in-context, that the budget is
//!    spent (with the exact numbers and its remaining tokens) and to stop
//!    starting new work and summarise where it got to.
//! 3. It gets [`BUDGET_WRAPUP_GRACE`] iterations to do that; if it ignores the
//!    instruction and keeps calling tools, the turn ends anyway with a
//!    "budget reached — here's where I am, continue?" message.
//!
//! Nothing here cancels an in-flight tool call: the check runs at the top of
//! each iteration, which is the same safe boundary the other loop guards use.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::providers::base::ProviderUsage;
use crate::providers::pricing::estimate_cost_usd;

/// Fraction of a limit at which the user gets a one-time heads-up.
pub const BUDGET_WARN_FRACTION: f64 = 0.8;

/// Iterations the model gets to deliver its wrap-up after the budget is spent,
/// before the turn is ended for it. Mirrors [`super::stall::STALL_WRAPUP_GRACE`].
pub const BUDGET_WRAPUP_GRACE: u32 = 2;

/// The per-reply ceiling. Every axis is independent and optional; `None` means
/// "unbounded on this axis". An all-`None` budget is inert (see
/// [`ReplyBudget::is_set`]) and costs the loop nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ReplyBudget {
    /// Wall-clock seconds from the start of the reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_seconds: Option<u64>,
    /// Provider-reported tokens summed across every turn of the reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    /// Estimated spend in dollars. Ignored when the model's price is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_usd: Option<f64>,
}

impl ReplyBudget {
    /// True when at least one axis is bounded.
    pub fn is_set(&self) -> bool {
        self.max_seconds.is_some() || self.max_tokens.is_some() || self.max_usd.is_some()
    }

    /// Read the global (config/env) budget. Absent or non-positive keys leave
    /// the axis unbounded, so a stray `0` can never wedge every reply at zero.
    pub fn from_config(config: &Config) -> Self {
        Self {
            max_seconds: config
                .get_param::<u64>("BIOROUTER_REPLY_BUDGET_SECONDS")
                .ok()
                .filter(|v| *v > 0),
            max_tokens: config
                .get_param::<u64>("BIOROUTER_REPLY_BUDGET_TOKENS")
                .ok()
                .filter(|v| *v > 0),
            max_usd: config
                .get_param::<f64>("BIOROUTER_REPLY_BUDGET_USD")
                .ok()
                .filter(|v| *v > 0.0),
        }
    }

    /// Per-axis merge of the session's budget over the global one: a session may
    /// tighten (or loosen) one axis without having to restate the others.
    pub fn resolve(session: Option<Self>, config: &Config) -> Self {
        let global = Self::from_config(config);
        let session = session.unwrap_or_default();
        Self {
            max_seconds: session.max_seconds.or(global.max_seconds),
            max_tokens: session.max_tokens.or(global.max_tokens),
            max_usd: session.max_usd.or(global.max_usd),
        }
    }
}

/// What the loop should do about the budget right now.
#[derive(Debug, Clone, PartialEq)]
pub enum BudgetAction {
    /// Under budget (or unbounded): carry on.
    Proceed,
    /// First crossing of [`BUDGET_WARN_FRACTION`] on some axis — tell the user once.
    Warn(BudgetSnapshot),
    /// A limit is spent: tell the model to wrap up (once), then hard-stop after
    /// the grace window.
    Exceeded(BudgetSnapshot),
}

/// A point-in-time reading of the reply's spend, for the meter and the messages.
/// Serializable so a GUI meter can be fed straight from it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetSnapshot {
    /// Wall-clock seconds since the reply started.
    pub elapsed_seconds: u64,
    /// Provider-reported tokens billed by this reply so far.
    pub tokens: u64,
    /// Estimated dollars spent, when every turn so far could be priced.
    pub usd: Option<f64>,
    /// The limits in force.
    pub limits: ReplyBudget,
    /// Fraction (0.0-1.0+) of the *tightest* bounded axis that is used up.
    pub fraction: f64,
    /// The axis that is furthest along ("time", "tokens", "cost"), if any is bounded.
    pub axis: Option<&'static str>,
}

impl BudgetSnapshot {
    /// Tokens left on the token axis, if it is bounded. Handed to the model on
    /// the wrap-up instruction so it can size its final answer.
    pub fn remaining_tokens(&self) -> Option<u64> {
        self.limits
            .max_tokens
            .map(|max| max.saturating_sub(self.tokens))
    }

    /// One-line "spent X of Y" for whichever axes are bounded.
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(max) = self.limits.max_seconds {
            parts.push(format!("{}s of {}s", self.elapsed_seconds, max));
        }
        if let Some(max) = self.limits.max_tokens {
            parts.push(format!("{} of {} tokens", self.tokens, max));
        }
        if let (Some(max), Some(usd)) = (self.limits.max_usd, self.usd) {
            parts.push(format!("${usd:.2} of ${max:.2}"));
        }
        if parts.is_empty() {
            format!("{}s, {} tokens", self.elapsed_seconds, self.tokens)
        } else {
            parts.join(", ")
        }
    }
}

/// Cumulative spend of one reply, checked at each iteration boundary.
///
/// Pure and clock-injectable: [`Self::check_at`] takes the elapsed time, so the
/// staged behaviour is unit-testable without sleeping.
#[derive(Debug)]
pub struct BudgetTracker {
    limits: ReplyBudget,
    tokens: u64,
    usd: f64,
    /// False once any turn's model could not be priced — the dollar figure is
    /// then an undercount, so we never enforce (or display) it.
    priced: bool,
    warned: bool,
    exceeded: bool,
}

impl BudgetTracker {
    pub fn new(limits: ReplyBudget) -> Self {
        Self {
            limits,
            tokens: 0,
            usd: 0.0,
            priced: true,
            warned: false,
            exceeded: false,
        }
    }

    /// True when at least one axis is bounded. The loop skips all budget work
    /// (including the pricing lookup) when this is false.
    pub fn is_active(&self) -> bool {
        self.limits.is_set()
    }

    /// True once the model has been told to wrap up.
    pub fn has_stopped(&self) -> bool {
        self.exceeded
    }

    /// Fold one turn's provider usage into the running totals.
    ///
    /// `provider` is the provider name (e.g. `anthropic`) and the model comes
    /// from the usage itself, so a lead/worker swap mid-reply is priced per turn
    /// at the model that actually ran.
    pub fn record_usage(&mut self, provider: &str, usage: &ProviderUsage) {
        let input = usage.usage.input_tokens.unwrap_or(0).max(0) as u64;
        let output = usage.usage.output_tokens.unwrap_or(0).max(0) as u64;
        // The larger of what the call billed and what it occupied. For an API
        // provider one call is one request and the two agree. A coding-agent
        // provider runs a whole child loop in one call: its billed buckets sum
        // every request, while `total_tokens` is the context gauge, the LAST
        // request alone (`claude_stream`, rule 6; Codex's `tokenUsage.last`).
        // Counting the gauge would let a reply's spend past the cap.
        let occupied = usage
            .usage
            .total_tokens
            .filter(|t| *t > 0)
            .map_or(input + output, |t| t as u64);
        let billed = usage
            .usage
            .billed_total()
            .map_or(0, |t| u64::try_from(t).unwrap_or(0));
        self.tokens = self.tokens.saturating_add(occupied.max(billed));

        // Only the dollar axis needs pricing, and only when it is bounded.
        if self.limits.max_usd.is_none() {
            return;
        }
        match estimate_cost_usd(provider, &usage.model, input, output) {
            Some(cost) => self.usd += cost,
            None => self.priced = false,
        }
    }

    /// The reading as of `elapsed`.
    pub fn snapshot_at(&self, elapsed: Duration) -> BudgetSnapshot {
        let elapsed_seconds = elapsed.as_secs();
        let usd = self.priced.then_some(self.usd);

        let mut fraction = 0.0_f64;
        let mut axis: Option<&'static str> = None;
        let mut consider = |used: f64, max: f64, name: &'static str| {
            if max > 0.0 {
                let f = used / max;
                if axis.is_none() || f > fraction {
                    fraction = f;
                    axis = Some(name);
                }
            }
        };
        if let Some(max) = self.limits.max_seconds {
            consider(elapsed.as_secs_f64(), max as f64, "time");
        }
        if let Some(max) = self.limits.max_tokens {
            consider(self.tokens as f64, max as f64, "tokens");
        }
        if let (Some(max), Some(usd)) = (self.limits.max_usd, usd) {
            consider(usd, max, "cost");
        }

        BudgetSnapshot {
            elapsed_seconds,
            tokens: self.tokens,
            usd,
            limits: self.limits,
            fraction,
            axis,
        }
    }

    /// Staged verdict for this iteration. Each stage fires at most once:
    /// `Warn` only on the first crossing of [`BUDGET_WARN_FRACTION`], `Exceeded`
    /// only on the first overrun (the loop owns the grace window after that).
    pub fn check_at(&mut self, elapsed: Duration) -> BudgetAction {
        if !self.is_active() || self.exceeded {
            return BudgetAction::Proceed;
        }
        let snapshot = self.snapshot_at(elapsed);
        if snapshot.fraction >= 1.0 {
            self.exceeded = true;
            // A budget that blows straight past the warn line never shows a
            // half-spent meter; don't warn afterwards either.
            self.warned = true;
            return BudgetAction::Exceeded(snapshot);
        }
        if !self.warned && snapshot.fraction >= BUDGET_WARN_FRACTION {
            self.warned = true;
            return BudgetAction::Warn(snapshot);
        }
        BudgetAction::Proceed
    }
}

/// Model-visible instruction injected when the budget is spent. Deliberately
/// concrete (it names the numbers and the tokens left) so the model wraps up
/// instead of opening a new line of work.
pub fn wrapup_instruction(snapshot: &BudgetSnapshot) -> String {
    let remaining = snapshot
        .remaining_tokens()
        .map_or_else(String::new, |left| {
            format!(" You have about {left} tokens left in this budget.")
        });
    format!(
        "[budget] This reply has used its budget ({}). Stop starting new work: \
         run no further tools unless one is strictly required to answer, and \
         reply now with what you have: what you did, what you found, and the \
         exact next step you would take.{remaining} The user can ask you to \
         continue, which starts a fresh budget.",
        snapshot.describe()
    )
}

/// User-visible message when the turn ends on the budget. Says plainly that the
/// stop is a budget stop, not a completion, and how to keep going or raise it.
pub fn stopped_message(snapshot: &BudgetSnapshot) -> String {
    format!(
        "I've reached the budget for this reply ({}), so I'm stopping here, not \
         because the task is necessarily complete. Would you like me to continue? \
         (raise or clear the cap with `BIOROUTER_REPLY_BUDGET_SECONDS` / \
         `BIOROUTER_REPLY_BUDGET_TOKENS` / `BIOROUTER_REPLY_BUDGET_USD`.)",
        snapshot.describe()
    )
}

/// The progress meter: a single heads-up as the reply nears its ceiling.
pub fn progress_note(snapshot: &BudgetSnapshot) -> String {
    format!(
        "⏳ Budget {:.0}% used ({}); I'll wrap up if it runs out.",
        (snapshot.fraction * 100.0).min(100.0),
        snapshot.describe()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::base::Usage;

    /// A config backed by throwaway files, so a test never reads (or writes) the
    /// user's real `config.yaml`.
    fn test_config() -> Config {
        let config_file = tempfile::NamedTempFile::new().unwrap();
        let secrets_file = tempfile::NamedTempFile::new().unwrap();
        Config::new_with_file_secrets(config_file.path(), secrets_file.path()).unwrap()
    }

    fn usage(model: &str, input: i32, output: i32) -> ProviderUsage {
        ProviderUsage::new(
            model.to_string(),
            Usage::new(Some(input), Some(output), Some(input + output)),
        )
    }

    #[test]
    fn an_unset_budget_is_inert() {
        let mut tracker = BudgetTracker::new(ReplyBudget::default());
        assert!(!tracker.is_active());
        tracker.record_usage("anthropic", &usage("claude", 1_000_000, 1_000_000));
        assert_eq!(
            tracker.check_at(Duration::from_secs(86_400)),
            BudgetAction::Proceed
        );
    }

    #[test]
    fn the_token_axis_warns_once_then_stops() {
        let mut tracker = BudgetTracker::new(ReplyBudget {
            max_tokens: Some(1_000),
            ..Default::default()
        });
        tracker.record_usage("anthropic", &usage("claude", 400, 50));
        assert_eq!(
            tracker.check_at(Duration::ZERO),
            BudgetAction::Proceed,
            "45% of the token budget is not worth a word"
        );

        tracker.record_usage("anthropic", &usage("claude", 300, 50));
        // 850/1000 — over the warn line.
        let BudgetAction::Warn(snapshot) = tracker.check_at(Duration::ZERO) else {
            panic!("expected a warn at 85% of the token budget");
        };
        assert_eq!(snapshot.tokens, 800);
        assert_eq!(snapshot.axis, Some("tokens"));
        assert_eq!(snapshot.remaining_tokens(), Some(200));

        // The warning is one-shot: a second check at the same spend is silent.
        assert_eq!(tracker.check_at(Duration::ZERO), BudgetAction::Proceed);

        tracker.record_usage("anthropic", &usage("claude", 300, 0));
        let BudgetAction::Exceeded(snapshot) = tracker.check_at(Duration::ZERO) else {
            panic!("expected the token budget to be spent");
        };
        assert_eq!(snapshot.tokens, 1_100);
        assert_eq!(snapshot.remaining_tokens(), Some(0));
        assert!(tracker.has_stopped());
        // And it only fires once — the loop owns the grace window from here.
        assert_eq!(tracker.check_at(Duration::ZERO), BudgetAction::Proceed);
    }

    #[test]
    fn the_wall_clock_axis_fires_without_any_usage() {
        let mut tracker = BudgetTracker::new(ReplyBudget {
            max_seconds: Some(60),
            ..Default::default()
        });
        assert_eq!(
            tracker.check_at(Duration::from_secs(30)),
            BudgetAction::Proceed
        );
        assert!(matches!(
            tracker.check_at(Duration::from_secs(50)),
            BudgetAction::Warn(_)
        ));
        let BudgetAction::Exceeded(snapshot) = tracker.check_at(Duration::from_secs(61)) else {
            panic!("a 61s reply must blow a 60s budget even with no tokens reported");
        };
        assert_eq!(snapshot.axis, Some("time"));
        assert_eq!(snapshot.elapsed_seconds, 61);
        // No token axis set, so nothing to promise the model.
        assert_eq!(snapshot.remaining_tokens(), None);
    }

    #[test]
    fn a_budget_blown_in_one_turn_skips_the_warning() {
        let mut tracker = BudgetTracker::new(ReplyBudget {
            max_tokens: Some(100),
            ..Default::default()
        });
        tracker.record_usage("anthropic", &usage("claude", 5_000, 1_000));
        assert!(matches!(
            tracker.check_at(Duration::ZERO),
            BudgetAction::Exceeded(_)
        ));
        assert_eq!(tracker.check_at(Duration::ZERO), BudgetAction::Proceed);
    }

    #[test]
    fn the_dollar_axis_prices_a_known_model() {
        // groq/llama-3.1-8b-instant: $0.05/M in, $0.08/M out.
        let mut tracker = BudgetTracker::new(ReplyBudget {
            max_usd: Some(0.10),
            ..Default::default()
        });
        tracker.record_usage("groq", &usage("llama-3.1-8b-instant", 1_000_000, 0));
        let snapshot = tracker.snapshot_at(Duration::ZERO);
        assert!(
            (snapshot.usd.unwrap() - 0.05).abs() < 1e-9,
            "1M input tokens at $0.05/M is $0.05, got {:?}",
            snapshot.usd
        );
        assert_eq!(snapshot.axis, Some("cost"));
        assert_eq!(tracker.check_at(Duration::ZERO), BudgetAction::Proceed);

        tracker.record_usage("groq", &usage("llama-3.1-8b-instant", 1_000_000, 0));
        assert!(matches!(
            tracker.check_at(Duration::ZERO),
            BudgetAction::Exceeded(_)
        ));
    }

    #[test]
    fn an_unpriceable_model_never_trips_the_dollar_axis() {
        let mut tracker = BudgetTracker::new(ReplyBudget {
            max_usd: Some(0.000_001),
            ..Default::default()
        });
        tracker.record_usage(
            "some-unknown-provider",
            &usage("mystery-model", 10_000_000, 10_000_000),
        );
        let snapshot = tracker.snapshot_at(Duration::ZERO);
        assert_eq!(
            snapshot.usd, None,
            "an unpriceable turn must undercount visibly, not silently"
        );
        assert_eq!(
            tracker.check_at(Duration::ZERO),
            BudgetAction::Proceed,
            "we must not stop a reply on a cost we cannot compute"
        );
    }

    /// A coding-agent call reports the context gauge (its last request) in
    /// `total_tokens` and the whole child loop's spend in the billed buckets.
    /// The token cap counts the spend.
    #[test]
    fn a_coding_agent_call_counts_what_it_billed_not_its_last_request() {
        let mut tracker = BudgetTracker::new(ReplyBudget {
            max_tokens: Some(100_000),
            ..Default::default()
        });
        // The recorded three-request Claude Code turn: 89,409 billed, 30,362 held.
        let usage = ProviderUsage::new(
            "claude-opus-5-5".to_string(),
            Usage::new(Some(6), Some(258), Some(30_362)).with_cache(Some(87_563), Some(1_582)),
        );
        tracker.record_usage("claude_code", &usage);
        assert_eq!(tracker.snapshot_at(Duration::ZERO).tokens, 89_409);
    }

    #[test]
    fn the_tightest_axis_wins_the_meter() {
        let mut tracker = BudgetTracker::new(ReplyBudget {
            max_seconds: Some(600),
            max_tokens: Some(1_000),
            ..Default::default()
        });
        tracker.record_usage("anthropic", &usage("claude", 900, 0));
        // 90% of tokens vs 10% of the clock.
        let snapshot = tracker.snapshot_at(Duration::from_secs(60));
        assert_eq!(snapshot.axis, Some("tokens"));
        assert!((snapshot.fraction - 0.9).abs() < 1e-9);
        assert!(snapshot.describe().contains("900 of 1000 tokens"));
        assert!(snapshot.describe().contains("60s of 600s"));
    }

    #[test]
    fn a_session_budget_overrides_the_global_one_per_axis() {
        let config = test_config();
        config
            .set_param("BIOROUTER_REPLY_BUDGET_SECONDS", 900u64)
            .unwrap();
        config
            .set_param("BIOROUTER_REPLY_BUDGET_TOKENS", 500_000u64)
            .unwrap();

        let resolved = ReplyBudget::resolve(
            Some(ReplyBudget {
                max_tokens: Some(1_000),
                ..Default::default()
            }),
            &config,
        );
        assert_eq!(resolved.max_tokens, Some(1_000), "session wins its axis");
        assert_eq!(
            resolved.max_seconds,
            Some(900),
            "and inherits the axes it left alone"
        );
        assert_eq!(resolved.max_usd, None);
        assert!(resolved.is_set());
    }

    #[test]
    fn no_config_means_no_budget() {
        let config = test_config();
        let resolved = ReplyBudget::resolve(None, &config);
        assert!(
            !resolved.is_set(),
            "the budget must be off unless someone asks for it"
        );
    }

    #[test]
    fn a_zero_limit_is_ignored_rather_than_wedging_every_reply() {
        let config = test_config();
        config
            .set_param("BIOROUTER_REPLY_BUDGET_TOKENS", 0u64)
            .unwrap();
        assert_eq!(ReplyBudget::from_config(&config).max_tokens, None);
    }

    #[test]
    fn the_messages_name_the_numbers() {
        let tracker = BudgetTracker::new(ReplyBudget {
            max_tokens: Some(1_000),
            ..Default::default()
        });
        let snapshot = tracker.snapshot_at(Duration::from_secs(5));
        assert!(wrapup_instruction(&snapshot).contains("0 of 1000 tokens"));
        assert!(wrapup_instruction(&snapshot).contains("1000 tokens left"));
        assert!(stopped_message(&snapshot).contains("0 of 1000 tokens"));
        assert!(progress_note(&snapshot).contains("0% used"));
    }
}
