//! Usage limits: a per-chat limit, a default per-chat limit, and a monthly
//! limit over all usage, each in billed tokens or priced dollars.
//!
//! * **Chat limit.** Stored on the chat's row in `extension_data` under
//!   `("usage_limit", "v1")`. The key is tri-state: absent means the chat
//!   inherits the default, `{"mode":"unlimited"}` lifts every per-chat limit
//!   for this chat (even when a default is set), and `{"mode":"limit", ...}`
//!   is the chat's own limit, which replaces the default on both axes.
//! * **Default per-chat limit and monthly limit.** The `usage` section of
//!   `config.yaml` ([`crate::config::usage::UsageLimits`]).
//!
//! Resolution is lazy, at check time: `effective = chat override ?? default`,
//! so changing the default changes every chat without its own limit.
//!
//! A chat's spend includes its subagents' (`SessionManager::spend_snapshot`),
//! and a subagent is checked against its parent chat's limit. The month is the
//! LOCAL calendar month, the same rule as the usage summary.
//!
//! A dollar limit over a span where nothing is priced never fires: unpriced
//! dollars cannot be compared with a limit, and `$0` is not "within budget".
//!
//! The reply loop asks [`check`] before every model request and ends the turn
//! with no further provider call once a limit is reached. See
//! `docs/agent-loop/usage-limits.md`.

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::usage::UsageLimits;
use crate::session::{SessionManager, SessionType, SpendSnapshot};

pub const STATE_KEY: &str = "usage_limit";
pub const STATE_VERSION: &str = "v1";

/// Fraction of a limit at which the once-per-reply warning is shown.
pub const WARN_FRACTION: f64 = 0.8;

/// The start of every stop notice for a per-chat limit (its own or the default).
///
/// ⚠ Mirrored in `ui/desktop/src/components/conversation/usageLimitNotice.ts`;
/// a test below reads that file. Change both or neither.
pub const CHAT_LIMIT_REACHED_PREFIX: &str = "Usage limit reached for this chat: ";
/// The start of every stop notice for the monthly limit. Mirrored like
/// [`CHAT_LIMIT_REACHED_PREFIX`].
pub const MONTH_LIMIT_REACHED_PREFIX: &str = "Monthly usage limit reached: ";
/// The start of the 80% warning for a per-chat limit. Mirrored like
/// [`CHAT_LIMIT_REACHED_PREFIX`].
pub const CHAT_LIMIT_WARNING_PREFIX: &str = "This chat is near its usage limit: ";
/// The start of the 80% warning for the monthly limit. Mirrored like
/// [`CHAT_LIMIT_REACHED_PREFIX`].
pub const MONTH_LIMIT_WARNING_PREFIX: &str = "Near the monthly usage limit: ";

/// Why a scheduled run was not started at the monthly limit (the scheduler's
/// `ScheduledRunRefused`, which gives the firing back to `max_runs`).
pub const SCHEDULED_RUN_MONTH_LIMIT: &str = "Monthly usage limit reached: this scheduled run \
     was not started. To run it, raise or lift the monthly limit in Settings > App > Usage.";

/// Why a subagent was not started: the chat or the month is at its limit.
pub const SUBAGENT_LIMIT_REFUSAL: &str = "A usage limit is reached, so no subagent was \
     started. Do not retry: tell the user, who can raise or lift the limit.";

/// A chat's own limit, as stored. Absence of the key is "inherit".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ChatLimitOverride {
    /// No per-chat limit for this chat, whatever the default says.
    Unlimited,
    /// This chat's own limit. An axis left `None` is unlimited for this chat.
    Limit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_usd: Option<f64>,
    },
}

impl ChatLimitOverride {
    /// A limit with no positive axis is the same as unlimited.
    pub fn normalized(self) -> Self {
        match self {
            ChatLimitOverride::Limit {
                max_tokens,
                max_usd,
            } => {
                let max_tokens = max_tokens.filter(|v| *v > 0);
                let max_usd = max_usd.filter(|v| v.is_finite() && *v > 0.0);
                if max_tokens.is_none() && max_usd.is_none() {
                    ChatLimitOverride::Unlimited
                } else {
                    ChatLimitOverride::Limit {
                        max_tokens,
                        max_usd,
                    }
                }
            }
            other => other,
        }
    }
}

fn decode(value: serde_json::Value) -> Result<ChatLimitOverride> {
    serde_json::from_value(value)
        .context("this chat's stored usage limit (usage_limit/v1) is unreadable")
}

/// The chat's stored override. `Ok(None)` is "inherit"; an unreadable value is
/// an `Err`, so a caller decides what a corrupt value means.
pub async fn for_session(
    session_manager: &SessionManager,
    session_id: &str,
) -> Result<Option<ChatLimitOverride>> {
    match session_manager
        .get_extension_state(session_id, STATE_KEY, STATE_VERSION)
        .await
        .with_context(|| format!("reading the usage limit of {session_id}"))?
    {
        Some(value) => decode(value).map(Some),
        None => Ok(None),
    }
}

/// [`for_session`] for the enforcement path: a read error or an unreadable
/// value inherits the default (logged), so a damaged row is never more
/// permissive than the configured default and a fresh write repairs it.
pub async fn for_session_lenient(
    session_manager: &SessionManager,
    session_id: &str,
) -> Option<ChatLimitOverride> {
    match for_session(session_manager, session_id).await {
        Ok(stored) => stored,
        Err(e) => {
            tracing::warn!(
                session_id,
                "usage limit override ignored, using the default: {e:#}"
            );
            None
        }
    }
}

/// Store the chat's override; `None` returns the chat to the default. The write
/// replaces whatever was stored, a corrupt value included. It is a deliberate
/// change to the chat, so it stamps `updated_at` (coordinator ruling 16).
///
/// Returns `Ok(false)` when the chat does not exist.
pub async fn set_for_session(
    session_manager: &SessionManager,
    session_id: &str,
    value: Option<ChatLimitOverride>,
) -> Result<bool> {
    let next = match value {
        Some(value) => serde_json::to_value(value.normalized())?,
        None => serde_json::Value::Null,
    };
    let stored = session_manager
        .update_extension_state(session_id, STATE_KEY, STATE_VERSION, move |_| Ok(next))
        .await?;
    Ok(stored.is_some())
}

/// Where a chat's effective per-chat limit came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitSource {
    /// The chat's own override (a limit, or an explicit unlimited).
    Chat,
    /// The default per-chat limit in Settings.
    Default,
    /// No per-chat limit at all.
    None,
}

/// The limits in force for one chat at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectiveLimits {
    pub chat_tokens: Option<u64>,
    pub chat_usd: Option<f64>,
    pub source: LimitSource,
    pub month_tokens: Option<u64>,
    pub month_usd: Option<f64>,
}

impl EffectiveLimits {
    pub fn is_unlimited(&self) -> bool {
        self.chat_tokens.is_none()
            && self.chat_usd.is_none()
            && self.month_tokens.is_none()
            && self.month_usd.is_none()
    }

    /// Whether any dollar limit is in force, so the spend read must price.
    pub fn needs_usd(&self) -> bool {
        self.chat_usd.is_some() || self.month_usd.is_some()
    }
}

/// Compose a chat's override with the configured defaults and monthly limits.
pub fn resolve(chat: Option<ChatLimitOverride>, config: &UsageLimits) -> EffectiveLimits {
    let (chat_tokens, chat_usd, source) = match chat.map(ChatLimitOverride::normalized) {
        Some(ChatLimitOverride::Unlimited) => (None, None, LimitSource::Chat),
        Some(ChatLimitOverride::Limit {
            max_tokens,
            max_usd,
        }) => (max_tokens, max_usd, LimitSource::Chat),
        None => {
            let tokens = config.default_session_tokens();
            let usd = config.default_session_usd();
            let source = if tokens.is_some() || usd.is_some() {
                LimitSource::Default
            } else {
                LimitSource::None
            };
            (tokens, usd, source)
        }
    };
    EffectiveLimits {
        chat_tokens,
        chat_usd,
        source,
        month_tokens: config.monthly_tokens(),
        month_usd: config.monthly_usd(),
    }
}

/// Which limit a hit is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitScope {
    Chat,
    Month,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LimitAmount {
    Tokens { used: u64, limit: u64 },
    Usd { used: f64, limit: f64 },
}

impl LimitAmount {
    fn fraction(&self) -> f64 {
        match *self {
            LimitAmount::Tokens { used, limit } => used as f64 / limit as f64,
            LimitAmount::Usd { used, limit } => used / limit,
        }
    }

    fn used_text(&self) -> String {
        match *self {
            LimitAmount::Tokens { used, .. } => format!("{} tokens", group_thousands(used)),
            LimitAmount::Usd { used, .. } => format!("${used:.2}"),
        }
    }

    fn limit_text(&self) -> String {
        match *self {
            LimitAmount::Tokens { limit, .. } => format!("{} tokens", group_thousands(limit)),
            LimitAmount::Usd { limit, .. } => format!("${limit:.2}"),
        }
    }

    /// `"tokens"` or `"usd"`, for logs and loop-safety events.
    pub fn axis(&self) -> &'static str {
        match self {
            LimitAmount::Tokens { .. } => "tokens",
            LimitAmount::Usd { .. } => "usd",
        }
    }
}

/// One limit at or near its ceiling.
#[derive(Debug, Clone, PartialEq)]
pub struct LimitHit {
    pub scope: LimitScope,
    /// For a chat hit, whether the limit is the chat's own or the default.
    pub source: LimitSource,
    pub amount: LimitAmount,
    /// The local month the month figures cover, `YYYY-MM`.
    pub month: String,
}

/// Every limit in force compared with the spend, month first: a monthly stop
/// is the one a person most needs to know about, since lifting a chat limit
/// would not help.
fn hits(limits: &EffectiveLimits, spend: &SpendSnapshot) -> Vec<LimitHit> {
    let mut out = Vec::new();
    let month_tokens = spend.month_tokens.max(0) as u64;
    let chat_tokens = spend.chat_tokens.max(0) as u64;
    let mut push = |scope, source, amount| {
        out.push(LimitHit {
            scope,
            source,
            amount,
            month: spend.month.clone(),
        })
    };
    if let Some(limit) = limits.month_tokens {
        push(
            LimitScope::Month,
            LimitSource::None,
            LimitAmount::Tokens {
                used: month_tokens,
                limit,
            },
        );
    }
    if let (Some(limit), Some(used)) = (limits.month_usd, spend.month_usd) {
        push(
            LimitScope::Month,
            LimitSource::None,
            LimitAmount::Usd { used, limit },
        );
    }
    if let Some(limit) = limits.chat_tokens {
        push(
            LimitScope::Chat,
            limits.source,
            LimitAmount::Tokens {
                used: chat_tokens,
                limit,
            },
        );
    }
    if let (Some(limit), Some(used)) = (limits.chat_usd, spend.chat_usd) {
        push(
            LimitScope::Chat,
            limits.source,
            LimitAmount::Usd { used, limit },
        );
    }
    out
}

/// The limit that is reached, if any, and otherwise the first one at or past
/// [`WARN_FRACTION`].
pub fn evaluate(
    limits: &EffectiveLimits,
    spend: &SpendSnapshot,
) -> (Option<LimitHit>, Option<LimitHit>) {
    let all = hits(limits, spend);
    let reached = all.iter().find(|hit| hit.amount.fraction() >= 1.0).cloned();
    let warning = if reached.is_some() {
        None
    } else {
        all.into_iter()
            .find(|hit| hit.amount.fraction() >= WARN_FRACTION)
    };
    (reached, warning)
}

/// Group a count's digits by thousands: `1234567` reads `1,234,567`.
fn group_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// The durable notice a turn stopped by `hit` ends on, and the notice a message
/// sent to a paused chat gets. Numbers and names of limits only, never content.
pub fn stop_notice(hit: &LimitHit) -> String {
    let used = hit.amount.used_text();
    let limit = hit.amount.limit_text();
    match (hit.scope, hit.source) {
        (LimitScope::Month, _) => format!(
            "{MONTH_LIMIT_REACHED_PREFIX}{used} used in {month}, of a monthly limit of {limit}. \
             To continue, raise or lift it in Settings > App > Usage. \
             The month resets on the 1st, local time.",
            month = hit.month,
        ),
        (LimitScope::Chat, LimitSource::Default) => format!(
            "{CHAT_LIMIT_REACHED_PREFIX}{used} used, of the default per-chat limit of {limit}. \
             To continue, raise or lift the default in Settings > App > Usage, \
             or set this chat's own limit from the cost at the lower right of the composer."
        ),
        (LimitScope::Chat, _) => format!(
            "{CHAT_LIMIT_REACHED_PREFIX}{used} used, of its limit of {limit}. \
             To continue, raise or lift the limit from the cost at the lower right of the composer."
        ),
    }
}

/// The once-per-reply heads-up at [`WARN_FRACTION`] of a limit.
pub fn warning_notice(hit: &LimitHit) -> String {
    let used = hit.amount.used_text();
    let limit = hit.amount.limit_text();
    let percent = (hit.amount.fraction() * 100.0).floor() as u64;
    match hit.scope {
        LimitScope::Month => format!(
            "{MONTH_LIMIT_WARNING_PREFIX}{used} used in {month}, of {limit} ({percent}%).",
            month = hit.month,
        ),
        LimitScope::Chat => {
            let whose = if hit.source == LimitSource::Default {
                "the default per-chat limit of"
            } else {
                "its limit of"
            };
            format!("{CHAT_LIMIT_WARNING_PREFIX}{used} used, of {whose} {limit} ({percent}%).")
        }
    }
}

/// What one check found.
#[derive(Debug, Clone)]
pub struct LimitCheck {
    /// The chat whose limit was checked: the chat itself, or a subagent's parent.
    pub billing_session_id: String,
    pub chat_override: Option<ChatLimitOverride>,
    pub effective: EffectiveLimits,
    /// `None` when no limit is in force (the spend was not read).
    pub spend: Option<SpendSnapshot>,
    pub reached: Option<LimitHit>,
    pub warning: Option<LimitHit>,
}

/// The chat a session's spend is held against: a subagent's parent chat, or
/// the session itself.
pub async fn billing_session_id(session_manager: &SessionManager, session_id: &str) -> String {
    match session_manager.get_session(session_id, false).await {
        Ok(session) if session.session_type == SessionType::SubAgent => session
            .parent_session_id
            .unwrap_or_else(|| session_id.to_string()),
        _ => session_id.to_string(),
    }
}

/// Check `session_id` against the configured limits.
pub async fn check(session_manager: &SessionManager, session_id: &str) -> Result<LimitCheck> {
    check_with(session_manager, session_id, &UsageLimits::load()).await
}

/// [`check`] against given configured limits.
pub async fn check_with(
    session_manager: &SessionManager,
    session_id: &str,
    config: &UsageLimits,
) -> Result<LimitCheck> {
    let billing = billing_session_id(session_manager, session_id).await;
    let chat_override = for_session_lenient(session_manager, &billing).await;
    let effective = resolve(chat_override, config);
    if effective.is_unlimited() {
        return Ok(LimitCheck {
            billing_session_id: billing,
            chat_override,
            effective,
            spend: None,
            reached: None,
            warning: None,
        });
    }
    let spend = session_manager
        .spend_snapshot(&billing, effective.needs_usd())
        .await?;
    let (reached, warning) = evaluate(&effective, &spend);
    Ok(LimitCheck {
        billing_session_id: billing,
        chat_override,
        effective,
        spend: Some(spend),
        reached,
        warning,
    })
}

/// The reached limit for `session_id`, if any. A failed read is logged and
/// treated as not reached: a limit is a spend guard, and a database hiccup
/// must not stop a chat the person has not capped.
pub async fn reached_limit(session_manager: &SessionManager, session_id: &str) -> Option<LimitHit> {
    match check(session_manager, session_id).await {
        Ok(found) => found.reached,
        Err(e) => {
            tracing::warn!(
                session_id,
                "usage limit check failed, not enforced this time: {e:#}"
            );
            None
        }
    }
}

/// A month-only check, for doors that are not tied to one chat's limit (a
/// scheduled run starts a new chat).
pub async fn reached_monthly_limit(session_manager: &SessionManager) -> Option<LimitHit> {
    reached_monthly_limit_with(session_manager, &UsageLimits::load()).await
}

/// [`reached_monthly_limit`] against given configured limits.
pub async fn reached_monthly_limit_with(
    session_manager: &SessionManager,
    config: &UsageLimits,
) -> Option<LimitHit> {
    let effective = resolve(Some(ChatLimitOverride::Unlimited), config);
    if effective.is_unlimited() {
        return None;
    }
    // The month figures do not depend on the chat named here.
    match session_manager
        .spend_snapshot("", effective.needs_usd())
        .await
    {
        Ok(spend) => evaluate(&effective, &spend).0,
        Err(e) => {
            tracing::warn!("monthly usage limit check failed, not enforced this time: {e:#}");
            None
        }
    }
}

/// Per-reply state on the agent: whether this reply already warned, and
/// whether it already stopped (so a second stop point adds no second notice).
#[derive(Debug, Default)]
pub struct ReplyLimitState {
    warned: AtomicBool,
    stopped: AtomicBool,
}

impl ReplyLimitState {
    pub fn begin_reply(&self) {
        self.warned.store(false, Ordering::SeqCst);
        self.stopped.store(false, Ordering::SeqCst);
    }

    /// `true` the first time it is called in a reply.
    pub fn take_warning(&self) -> bool {
        !self.warned.swap(true, Ordering::SeqCst)
    }

    /// `true` the first time it is called in a reply.
    pub fn take_stop(&self) -> bool {
        !self.stopped.swap(true, Ordering::SeqCst)
    }

    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spend(chat: i64, month: i64) -> SpendSnapshot {
        SpendSnapshot {
            chat_tokens: chat,
            month_tokens: month,
            month: "2026-11".to_string(),
            ..SpendSnapshot::default()
        }
    }

    async fn manager_and_session(temp: &tempfile::TempDir) -> (SessionManager, String) {
        let sm = SessionManager::new(temp.path().to_path_buf());
        let session = sm
            .create_session(
                temp.path().to_path_buf(),
                "limits".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        (sm, session.id)
    }

    #[tokio::test]
    async fn the_override_is_tri_state_on_the_row() {
        let temp = tempfile::tempdir().unwrap();
        let (sm, id) = manager_and_session(&temp).await;
        assert_eq!(for_session(&sm, &id).await.unwrap(), None);

        assert!(
            set_for_session(&sm, &id, Some(ChatLimitOverride::Unlimited))
                .await
                .unwrap()
        );
        assert_eq!(
            for_session(&sm, &id).await.unwrap(),
            Some(ChatLimitOverride::Unlimited)
        );

        let limit = ChatLimitOverride::Limit {
            max_tokens: Some(1000),
            max_usd: None,
        };
        set_for_session(&sm, &id, Some(limit)).await.unwrap();
        assert_eq!(for_session(&sm, &id).await.unwrap(), Some(limit));

        set_for_session(&sm, &id, None).await.unwrap();
        assert_eq!(for_session(&sm, &id).await.unwrap(), None);

        assert!(!set_for_session(&sm, "no-such-chat", None).await.unwrap());
    }

    #[tokio::test]
    async fn a_corrupt_override_is_an_error_and_inherits_on_the_enforcement_path() {
        let temp = tempfile::tempdir().unwrap();
        let (sm, id) = manager_and_session(&temp).await;
        sm.update_extension_state(&id, STATE_KEY, STATE_VERSION, |_| {
            Ok(serde_json::json!({ "mode": "sideways" }))
        })
        .await
        .unwrap();
        assert!(for_session(&sm, &id).await.is_err());
        assert_eq!(for_session_lenient(&sm, &id).await, None);

        // A fresh write repairs it.
        set_for_session(&sm, &id, Some(ChatLimitOverride::Unlimited))
            .await
            .unwrap();
        assert_eq!(
            for_session(&sm, &id).await.unwrap(),
            Some(ChatLimitOverride::Unlimited)
        );
    }

    #[test]
    fn a_limit_with_no_positive_axis_is_unlimited() {
        let zero = ChatLimitOverride::Limit {
            max_tokens: Some(0),
            max_usd: Some(-1.0),
        };
        assert_eq!(zero.normalized(), ChatLimitOverride::Unlimited);
    }

    #[test]
    fn resolution_prefers_the_chat_then_the_default() {
        let config = UsageLimits {
            default_session_token_limit: Some(500),
            default_session_dollar_limit: Some(2.0),
            monthly_token_limit: Some(10_000),
            ..UsageLimits::default()
        };

        let inherited = resolve(None, &config);
        assert_eq!(inherited.source, LimitSource::Default);
        assert_eq!(inherited.chat_tokens, Some(500));
        assert_eq!(inherited.chat_usd, Some(2.0));
        assert_eq!(inherited.month_tokens, Some(10_000));

        let lifted = resolve(Some(ChatLimitOverride::Unlimited), &config);
        assert_eq!(lifted.source, LimitSource::Chat);
        assert_eq!(lifted.chat_tokens, None);
        assert_eq!(lifted.chat_usd, None);
        // Lifting the chat limit never lifts the month.
        assert_eq!(lifted.month_tokens, Some(10_000));

        let own = resolve(
            Some(ChatLimitOverride::Limit {
                max_tokens: Some(50),
                max_usd: None,
            }),
            &config,
        );
        assert_eq!(own.source, LimitSource::Chat);
        assert_eq!(own.chat_tokens, Some(50));
        // The chat's own limit replaces the default on both axes.
        assert_eq!(own.chat_usd, None);

        let none = resolve(None, &UsageLimits::default());
        assert_eq!(none.source, LimitSource::None);
        assert!(none.is_unlimited());
    }

    #[test]
    fn the_month_is_reported_before_the_chat_and_warnings_start_at_80_percent() {
        let limits = EffectiveLimits {
            chat_tokens: Some(100),
            chat_usd: None,
            source: LimitSource::Chat,
            month_tokens: Some(1000),
            month_usd: None,
        };
        let (reached, warning) = evaluate(&limits, &spend(100, 1000));
        assert_eq!(reached.unwrap().scope, LimitScope::Month);
        assert!(warning.is_none());

        let (reached, _) = evaluate(&limits, &spend(100, 10));
        assert_eq!(reached.unwrap().scope, LimitScope::Chat);

        let (reached, warning) = evaluate(&limits, &spend(80, 10));
        assert!(reached.is_none());
        assert_eq!(warning.unwrap().scope, LimitScope::Chat);

        let (reached, warning) = evaluate(&limits, &spend(79, 10));
        assert!(reached.is_none() && warning.is_none());
    }

    #[test]
    fn an_unpriced_dollar_limit_never_fires() {
        let limits = EffectiveLimits {
            chat_tokens: None,
            chat_usd: Some(1.0),
            source: LimitSource::Chat,
            month_tokens: None,
            month_usd: Some(1.0),
        };
        let mut unpriced = spend(1_000_000_000, 1_000_000_000);
        unpriced.chat_usd = None;
        unpriced.month_usd = None;
        assert_eq!(evaluate(&limits, &unpriced), (None, None));

        let mut priced = unpriced.clone();
        priced.chat_usd = Some(1.5);
        let (reached, _) = evaluate(&limits, &priced);
        assert!(matches!(
            reached.unwrap().amount,
            LimitAmount::Usd { used, limit } if used == 1.5 && limit == 1.0
        ));
    }

    #[test]
    fn notices_name_the_limit_the_amount_and_where_to_change_it() {
        let hit = LimitHit {
            scope: LimitScope::Chat,
            source: LimitSource::Chat,
            amount: LimitAmount::Tokens {
                used: 1_020_000,
                limit: 1_000_000,
            },
            month: "2026-11".to_string(),
        };
        let text = stop_notice(&hit);
        assert!(text.starts_with(CHAT_LIMIT_REACHED_PREFIX));
        assert!(text.contains("1,020,000 tokens used"), "{text}");
        assert!(text.contains("1,000,000 tokens"), "{text}");
        assert!(text.contains("lower right of the composer"), "{text}");

        let default = LimitHit {
            source: LimitSource::Default,
            ..hit.clone()
        };
        assert!(stop_notice(&default).contains("Settings > App > Usage"));

        let month = LimitHit {
            scope: LimitScope::Month,
            source: LimitSource::None,
            amount: LimitAmount::Usd {
                used: 251.0,
                limit: 250.0,
            },
            month: "2026-11".to_string(),
        };
        let text = stop_notice(&month);
        assert!(text.starts_with(MONTH_LIMIT_REACHED_PREFIX));
        assert!(text.contains("$251.00 used in 2026-11"), "{text}");
        assert!(text.contains("$250.00"), "{text}");
        assert!(text.contains("Settings > App > Usage"), "{text}");

        assert!(warning_notice(&hit).starts_with(CHAT_LIMIT_WARNING_PREFIX));
        assert!(warning_notice(&month).starts_with(MONTH_LIMIT_WARNING_PREFIX));

        for text in [
            stop_notice(&hit),
            stop_notice(&default),
            stop_notice(&month),
            warning_notice(&hit),
        ] {
            assert!(!text.contains('\u{2014}'), "no em dash in copy: {text}");
        }
    }

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1000), "1,000");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn the_reply_state_warns_and_stops_once() {
        let state = ReplyLimitState::default();
        assert!(state.take_warning());
        assert!(!state.take_warning());
        assert!(state.take_stop());
        assert!(!state.take_stop());
        assert!(state.stopped());
        state.begin_reply();
        assert!(!state.stopped());
        assert!(state.take_warning());
    }

    /// The desktop recognises these notices by prefix. Same pattern as
    /// `stopped_turn`'s mirror test.
    #[test]
    fn the_desktop_mirrors_the_notice_prefixes() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ui/desktop/src/components/conversation/usageLimitNotice.ts");
        let source = std::fs::read_to_string(&path).expect("usageLimitNotice.ts is readable");
        for (name, value) in [
            ("USAGE_LIMIT_CHAT_REACHED_PREFIX", CHAT_LIMIT_REACHED_PREFIX),
            (
                "USAGE_LIMIT_MONTH_REACHED_PREFIX",
                MONTH_LIMIT_REACHED_PREFIX,
            ),
            ("USAGE_LIMIT_CHAT_WARNING_PREFIX", CHAT_LIMIT_WARNING_PREFIX),
            (
                "USAGE_LIMIT_MONTH_WARNING_PREFIX",
                MONTH_LIMIT_WARNING_PREFIX,
            ),
        ] {
            let declaration = format!("export const {name} = '{value}';");
            assert!(
                source.contains(&declaration),
                "usageLimitNotice.ts must declare `{declaration}`"
            );
        }
    }
}
