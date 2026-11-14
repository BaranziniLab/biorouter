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
//! The reply loop asks [`check_next`] before every model request and ends the
//! turn with no further provider call once a limit is reached, or once the
//! spend so far plus an estimate of that request would pass it
//! ([`project_request`]), so the last request does not run whole past the
//! limit. See `docs/agent-loop/usage-limits.md`.
//!
//! ⚠ A check compares BILLED spend, which a request adds only once its
//! response lands. Requests running at the same time (parallel subagents of
//! one chat, or several chats against the monthly limit) do not see one
//! another, so together they can pass a limit by up to about one request
//! each. Nothing reserves a request's estimate while it runs.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::usage::UsageLimits;
use crate::session::{RequestUsage, SessionManager, SessionType, SpendSnapshot};

pub const STATE_KEY: &str = "usage_limit";
pub const STATE_VERSION: &str = "v1";

/// Which near-limit warnings a chat has already been given, so each is shown
/// once per crossing ([`announce_warning_once`]). Its own key, beside the
/// person's override, because the reply loop writes it.
pub const NOTICE_STATE_KEY: &str = "usage_limit_notice";
pub const NOTICE_STATE_VERSION: &str = "v1";

/// Fraction of a limit at which the near-limit warning is shown, once per
/// crossing.
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

/// Why a Crew task was not started at the monthly limit (`POST /crew/connections/{id}/runs`
/// and a chat's `tasks_ask`). A task runs in a chat of its own, so the month is the only limit
/// in force before it starts.
pub const CREW_TASK_MONTH_LIMIT: &str = "Monthly usage limit reached: this Crew task was not \
     started. To run it, raise or lift the monthly limit in Settings > App > Usage.";

/// Why a chat's `tasks_ask` started no task: the asking chat is at its own limit. The task's
/// spend is still its own task chat's and the month's, never the asking chat's.
pub const CREW_TASK_ASK_LIMIT_REFUSAL: &str = "A usage limit is reached for this chat, so no \
     Crew task was started. Do not retry: tell the user, who can raise or lift the limit in \
     Settings > App > Usage.";

/// How a Crew task that a usage limit stopped part way ends, in place of its result.
pub const CREW_TASK_STOPPED_AT_LIMIT: &str = "A usage limit stopped this Crew task before it \
     finished, so it posted no result. Its owner can raise or lift the limit in Settings > App \
     > Usage and ask again.";

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
    if stored.is_some() {
        // A changed limit is a new limit: its warning is due again.
        if let Err(e) = session_manager
            .update_extension_state(session_id, NOTICE_STATE_KEY, NOTICE_STATE_VERSION, |_| {
                Ok(serde_json::Value::Null)
            })
            .await
        {
            tracing::warn!(
                session_id,
                "could not reset the usage limit warnings: {e:#}"
            );
        }
    }
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

    /// What was used and the limit, in one precision, so the two never read
    /// alike unless they are ([`format_usd_pair`]).
    fn texts(&self) -> (String, String) {
        match *self {
            LimitAmount::Tokens { used, limit } => (tokens_text(used), tokens_text(limit)),
            LimitAmount::Usd { used, limit } => format_usd_pair(used, limit),
        }
    }

    /// An amount on this limit's axis: a token count or dollars.
    fn amount_text(&self, value: f64) -> String {
        match self {
            LimitAmount::Tokens { .. } => tokens_text(value.max(0.0).round() as u64),
            LimitAmount::Usd { .. } => format_usd(value),
        }
    }

    fn used(&self) -> f64 {
        match *self {
            LimitAmount::Tokens { used, .. } => used as f64,
            LimitAmount::Usd { used, .. } => used,
        }
    }

    fn limit(&self) -> f64 {
        match *self {
            LimitAmount::Tokens { limit, .. } => limit as f64,
            LimitAmount::Usd { limit, .. } => limit,
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
    /// Set when the limit is not reached yet but the next model request is
    /// estimated to pass it: the estimate, on this limit's axis (tokens or
    /// dollars). [`stop_notice`] then says so and how much is left.
    pub next_request: Option<f64>,
}

impl LimitHit {
    /// The key a near-limit warning is remembered by: the limit's scope, axis
    /// and value (and the month, for the monthly limit), so a changed limit
    /// warns again.
    fn notice_key(&self) -> String {
        limit_key(self.scope, &self.amount, &self.month)
    }
}

fn limit_key(scope: LimitScope, amount: &LimitAmount, month: &str) -> String {
    let scope = match scope {
        LimitScope::Chat => "chat".to_string(),
        LimitScope::Month => format!("month:{month}"),
    };
    match *amount {
        LimitAmount::Tokens { limit, .. } => format!("{scope}:tokens:{limit}"),
        LimitAmount::Usd { limit, .. } => format!("{scope}:usd:{limit}"),
    }
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
            next_request: None,
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

/// Every limit in force at or past [`WARN_FRACTION`], month first. Each is
/// warned about on its own ([`announce_warning_once`]), so a month already
/// warned about does not hide a chat limit crossing 80% later.
pub fn warnings(limits: &EffectiveLimits, spend: &SpendSnapshot) -> Vec<LimitHit> {
    hits(limits, spend)
        .into_iter()
        .filter(|hit| hit.amount.fraction() >= WARN_FRACTION)
        .collect()
}

/// The first limit in force, month first, that `next` would pass, given
/// that none is reached yet. A limit whose axis the estimate does not price
/// (dollars for an unpriced model) is not projected.
pub fn projected_hit(
    limits: &EffectiveLimits,
    spend: &SpendSnapshot,
    next: &RequestEstimate,
) -> Option<LimitHit> {
    hits(limits, spend).into_iter().find_map(|mut hit| {
        let estimate = match hit.amount {
            LimitAmount::Tokens { .. } => Some(next.tokens as f64),
            LimitAmount::Usd { .. } => next.usd,
        }?;
        if estimate > 0.0 && hit.amount.used() + estimate > hit.amount.limit() {
            hit.next_request = Some(estimate);
            Some(hit)
        } else {
            None
        }
    })
}

/// What the next model request is estimated to bill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RequestEstimate {
    /// Billed tokens: the whole prompt plus the output.
    pub tokens: u64,
    /// Priced dollars, `None` when the model has no price.
    pub usd: Option<f64>,
}

/// What the estimate of the next request is made from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NextRequestInputs {
    /// The provider and model the request goes to; the previous request's
    /// when `None`.
    pub provider: Option<String>,
    pub model: Option<String>,
    /// The context manager's size estimate of the request about to be sent
    /// (`context_mgmt::estimate_request_size`), when the caller holds it.
    pub prompt_estimate: Option<u64>,
    /// The same estimate made before the previous request of this reply,
    /// so the previous request's billed prompt can be scaled by the growth
    /// since (a tool result, the model's answer).
    pub previous_prompt_estimate: Option<u64>,
    /// The context manager's size estimate of what the request about to be
    /// sent carries after the previous request's answer: the new message, an
    /// attachment, a pasted document. Only the reply loop sees it; a reply's
    /// first request adds it to the previous request plus its answer.
    pub added_estimate: Option<u64>,
}

impl NextRequestInputs {
    /// What the composer, the prologue and a chat's title estimate the next
    /// request from: `provider`'s name and model, and no prompt size, so the
    /// estimate is the previous request plus its answer ([`hold`]).
    pub fn at_rest(provider: &dyn crate::providers::base::Provider) -> Self {
        Self {
            provider: Some(provider.get_name().to_string()),
            model: Some(provider.get_model_config().model_name),
            ..Default::default()
        }
    }
}

/// The token buckets the next request is expected to bill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectedRequest {
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub output_tokens: i64,
}

impl ProjectedRequest {
    pub fn prompt_tokens(&self) -> i64 {
        self.input_tokens + self.cache_read_tokens + self.cache_creation_tokens
    }

    pub fn billed_tokens(&self) -> i64 {
        self.prompt_tokens() + self.output_tokens
    }
}

/// The next request's buckets, pure.
///
/// * With an earlier request in this chat (`previous`): its billed prompt,
///   scaled by how much the context manager's estimate grew since it was
///   sent when both estimates are known (inside a reply). Otherwise (a
///   reply's first request) its prompt plus its output (the answer joins the
///   history) plus [`NextRequestInputs::added_estimate`], so a large new
///   message counts before it is sent. Its cache shares and its output carry
///   over; what was added is counted as fresh input.
/// * Before the chat's first request: the context manager's estimate of the
///   prompt, with the cache shares and output of the model's last request in
///   any chat (`model_basis`), or none.
pub fn project_request(
    previous: Option<&RequestUsage>,
    model_basis: Option<&RequestUsage>,
    inputs: &NextRequestInputs,
) -> Option<ProjectedRequest> {
    // `cached` is the part of the prompt the basis's cache shares apply to.
    let (prompt, cached, basis) = match previous.filter(|p| p.prompt_tokens() > 0) {
        Some(previous) => {
            let before = previous.prompt_tokens();
            match (inputs.prompt_estimate, inputs.previous_prompt_estimate) {
                (Some(now), Some(then)) if then > 0 => {
                    let prompt = ((before as f64) * (now as f64) / (then as f64)).ceil() as i64;
                    (prompt, prompt, Some(previous))
                }
                _ => {
                    // The added part is estimated on its own, not the whole
                    // request: the context manager's size over-counts by
                    // design, and only a ratio of two of them is trusted.
                    let resting = before + previous.output_tokens.max(0);
                    let added = inputs
                        .added_estimate
                        .and_then(|added| i64::try_from(added).ok())
                        .unwrap_or(0);
                    (resting.saturating_add(added), resting, Some(previous))
                }
            }
        }
        None => {
            let prompt = i64::try_from(inputs.prompt_estimate?).ok()?;
            (
                prompt,
                prompt,
                model_basis.filter(|b| b.prompt_tokens() > 0),
            )
        }
    };
    let prompt = prompt.max(1);
    let cached = cached.clamp(0, prompt);
    let share = |part: i64| match basis {
        Some(b) => {
            ((cached as f64) * (part.max(0) as f64) / (b.prompt_tokens() as f64)).round() as i64
        }
        None => 0,
    };
    let cache_read_tokens = basis.map_or(0, |b| share(b.cache_read_tokens));
    let cache_creation_tokens = basis.map_or(0, |b| share(b.cache_creation_tokens));
    let input_tokens = (prompt - cache_read_tokens - cache_creation_tokens).max(0);
    Some(ProjectedRequest {
        input_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        output_tokens: basis.map_or(0, |b| b.output_tokens.max(0)),
    })
}

/// [`project_request`] priced: read the chat's last request (or the model's,
/// before the chat's first), project the next one, and price it with the
/// model's rates (its long-context rate past the threshold) when `with_usd`.
/// `None` when there is nothing to estimate from.
pub async fn estimate_next_request(
    session_manager: &SessionManager,
    session_id: &str,
    inputs: &NextRequestInputs,
    with_usd: bool,
) -> Result<Option<RequestEstimate>> {
    let previous = session_manager.last_request_usage(session_id).await?;
    let provider = inputs
        .provider
        .clone()
        .or_else(|| previous.as_ref().and_then(|p| p.provider.clone()));
    let model = inputs
        .model
        .clone()
        .or_else(|| previous.as_ref().and_then(|p| p.model_id.clone()));
    let model_basis = match (&previous, &provider, &model) {
        (None, Some(provider), Some(model)) => {
            session_manager
                .last_request_usage_for_model(provider, model)
                .await?
        }
        _ => None,
    };
    let Some(next) = project_request(previous.as_ref(), model_basis.as_ref(), inputs) else {
        return Ok(None);
    };
    // The model the request goes to, else the one the previous request was
    // served by (a lead/worker pair names itself, not a priced model).
    let mut usd = None;
    if with_usd {
        let served = previous
            .as_ref()
            .and_then(|p| p.provider.clone().zip(p.model_id.clone()));
        for (provider, model) in provider.zip(model).into_iter().chain(served) {
            if let Some(cost) = price_request(&provider, &model, &next).await {
                usd = Some(cost);
                break;
            }
        }
    }
    Ok(Some(RequestEstimate {
        tokens: next.billed_tokens().max(0) as u64,
        usd,
    }))
}

/// `next` at `provider`'s `model` rates, its long-context rate past the
/// threshold; `None` when the model has no price.
async fn price_request(provider: &str, model: &str, next: &ProjectedRequest) -> Option<f64> {
    use crate::providers::pricing;
    let rates = pricing::resolved_provider_model_pricing(provider, model).await?;
    let tier = pricing::long_context_tier(provider, model);
    let long = tier
        .as_ref()
        .is_some_and(|t| t.applies_to_prompt(next.prompt_tokens()));
    Some(
        pricing::cost_with_tier(
            &rates,
            tier.as_ref(),
            long,
            next.input_tokens,
            next.output_tokens,
            next.cache_read_tokens,
            next.cache_creation_tokens,
        )
        .cost,
    )
}

/// How many decimals a dollar amount is printed with: two from $1 up, and
/// below it three significant digits but never fewer than three decimals (a
/// tenth of a cent), at most [`MAX_USD_DECIMALS`].
fn usd_decimals(amount: f64) -> usize {
    if amount.is_nan() || amount <= 0.0 || amount >= 1.0 {
        return 2;
    }
    let first_significant = (-amount.log10()).floor() as i64 + 1;
    ((first_significant + 2).max(3) as usize).min(MAX_USD_DECIMALS)
}

const MAX_USD_DECIMALS: usize = 6;

/// `amount` at `decimals`, the whole dollars grouped by thousands; with
/// `trim`, trailing zeros past the cents are dropped.
fn render_usd(amount: f64, decimals: usize, trim: bool) -> String {
    let fixed = format!("{:.*}", decimals, amount.max(0.0));
    let (whole, fraction) = fixed.split_once('.').unwrap_or((&fixed, ""));
    let mut fraction = fraction.to_string();
    if trim {
        while fraction.len() > 2 && fraction.ends_with('0') {
            fraction.pop();
        }
    }
    let whole = whole
        .parse::<u64>()
        .map(group_thousands)
        .unwrap_or_else(|_| whole.to_string());
    if fraction.is_empty() {
        format!("${whole}")
    } else {
        format!("${whole}.{fraction}")
    }
}

/// A dollar amount as every usage surface prints it: `$5.00`, `$1,250.00`,
/// `$0.18`, `$0.177`, `$0.0085`. Below $1 it keeps three significant digits,
/// so a sub-cent spend never reads `<$0.01` or `$0.01`.
///
/// ⚠ Mirrored by `formatUsd` in `ui/desktop/src/utils/formatUsd.ts`; both run
/// `formatUsdCases.json` beside it. Change both or neither.
pub fn format_usd(amount: f64) -> String {
    if !amount.is_finite() || amount <= 0.0 {
        return "$0.00".to_string();
    }
    if amount < 10f64.powi(-(MAX_USD_DECIMALS as i32)) {
        return format!("<${:.*}", MAX_USD_DECIMALS, 1e-6);
    }
    render_usd(amount, usd_decimals(amount), true)
}

/// Two amounts that are read side by side (used and its limit): each as
/// [`format_usd`] prints it, unless two different amounts would read alike,
/// in which case both take the fewest decimals that tell them apart.
pub fn format_usd_pair(a: f64, b: f64) -> (String, String) {
    let pair = (format_usd(a), format_usd(b));
    if pair.0 != pair.1 || a == b || !a.is_finite() || !b.is_finite() {
        return pair;
    }
    for decimals in 2..=MAX_USD_DECIMALS {
        let (x, y) = (
            render_usd(a, decimals, false),
            render_usd(b, decimals, false),
        );
        if x != y {
            return (x, y);
        }
    }
    pair
}

fn tokens_text(count: u64) -> String {
    if count == 1 {
        "1 token".to_string()
    } else {
        format!("{} tokens", group_thousands(count))
    }
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

/// What a projected stop adds: the estimate of the next request and what is
/// left, or, when one request alone is estimated past the whole limit, that no
/// request fits under it with this model.
fn next_request_sentence(hit: &LimitHit, limit_text: &str) -> Option<String> {
    let next = hit.next_request?;
    let estimate = hit.amount.amount_text(next);
    if next > hit.amount.limit() {
        return Some(format!(
            "One request is estimated at {estimate}, more than the whole limit of {limit_text}, \
             so no request fits under it."
        ));
    }
    if hit.scope == LimitScope::Month {
        // What is left would give away the month's total ([`stop_notice`]).
        return Some(format!(
            "The next request is estimated at {estimate}, more than is left of it, so it \
             would pass the limit."
        ));
    }
    let left = hit
        .amount
        .amount_text((hit.amount.limit() - hit.amount.used()).max(0.0));
    Some(format!(
        "The next request is estimated at {estimate} and {left} is left, so it would pass the \
         limit."
    ))
}

/// The durable notice a turn stopped by `hit` ends on, and the notice a message
/// sent to a paused chat gets. Numbers and names of limits only, never content.
/// A projected stop ([`LimitHit::next_request`]) also says what the next
/// request is estimated at and how much is left.
///
/// ⚠ A monthly notice names the limit and never the month's total. The total
/// sums every chat, private and deleted ones included, and the notice is
/// stored in whichever chat reached it, which may be a public one whose
/// transcript a caller holding only the daemon secret can read; the usage
/// routes give that total only to a caller with the user-action proof.
pub fn stop_notice(hit: &LimitHit) -> String {
    let (used, limit) = hit.amount.texts();
    let next = next_request_sentence(hit, &limit)
        .map(|sentence| format!(" {sentence}"))
        .unwrap_or_default();
    match (hit.scope, hit.source) {
        (LimitScope::Month, _) => format!(
            "{MONTH_LIMIT_REACHED_PREFIX}{state}{next} To continue, raise or lift it in \
             Settings > App > Usage. The month resets on the 1st, local time.",
            state = if hit.next_request.is_some() {
                format!("the monthly limit for {} is {limit}.", hit.month)
            } else {
                format!("the monthly limit of {limit} for {} is used up.", hit.month)
            },
        ),
        (LimitScope::Chat, LimitSource::Default) => format!(
            "{CHAT_LIMIT_REACHED_PREFIX}{used} used, of the default per-chat limit of {limit}.\
             {next} To continue, raise or lift the default in Settings > App > Usage, \
             or set this chat's own limit from the cost at the lower right of the composer."
        ),
        (LimitScope::Chat, _) => format!(
            "{CHAT_LIMIT_REACHED_PREFIX}{used} used, of its limit of {limit}.{next} \
             To continue, raise or lift the limit from the cost at the lower right of the composer."
        ),
    }
}

/// The heads-up at [`WARN_FRACTION`] of a limit, once per crossing. A monthly
/// warning names the limit, never the month's total ([`stop_notice`]).
pub fn warning_notice(hit: &LimitHit) -> String {
    let (used, limit) = hit.amount.texts();
    let percent = (hit.amount.fraction() * 100.0).floor() as u64;
    match hit.scope {
        LimitScope::Month => format!(
            "{MONTH_LIMIT_WARNING_PREFIX}usage in {month} is at least {threshold}% of the \
             monthly limit of {limit}.",
            month = hit.month,
            threshold = (WARN_FRACTION * 100.0).round() as u64,
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

/// Which near-limit warnings a chat has been given ([`NOTICE_STATE_KEY`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct NoticeRecord {
    #[serde(default)]
    warned: Vec<String>,
}

/// Which near-limit warning is due now, remembered on the billing chat so each
/// limit's is given once per crossing and a reload does not bring it back:
/// the first limit at or past [`WARN_FRACTION`], month first, whose crossing
/// has not been warned about yet ([`warnings`]). One per check; a second
/// limit near its ceiling is warned about at the next check.
///
/// Every check with limits in force also forgets the warnings of limits that
/// are no longer in force or are back under [`WARN_FRACTION`] (a raised or
/// changed limit), so the next crossing warns again. The record is written
/// only when it changes. `Ok(None)` when no warning is due.
pub async fn announce_warning_once(
    session_manager: &SessionManager,
    check: &LimitCheck,
) -> Result<Option<LimitHit>> {
    let Some(spend) = check.spend.as_ref() else {
        return Ok(None);
    };
    let billing = check.billing_session_id.as_str();
    let stored: NoticeRecord = match session_manager
        .get_extension_state(billing, NOTICE_STATE_KEY, NOTICE_STATE_VERSION)
        .await?
    {
        Some(value) => serde_json::from_value(value).unwrap_or_default(),
        None => NoticeRecord::default(),
    };
    let near = warnings(&check.effective, spend);
    let still_warned: Vec<String> = near.iter().map(LimitHit::notice_key).collect();
    let mut next = NoticeRecord {
        warned: stored
            .warned
            .iter()
            .filter(|key| still_warned.contains(key))
            .cloned()
            .collect(),
    };
    let due = if check.reached.is_none() {
        near.into_iter()
            .find(|hit| !next.warned.contains(&hit.notice_key()))
    } else {
        None
    };
    if let Some(hit) = &due {
        next.warned.push(hit.notice_key());
    }
    if next != stored {
        let value = serde_json::to_value(&next)?;
        session_manager
            .update_extension_state(billing, NOTICE_STATE_KEY, NOTICE_STATE_VERSION, move |_| {
                Ok(value)
            })
            .await?;
    }
    Ok(due)
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
    /// The next request's estimate, when [`check_next`] made one.
    pub next_request: Option<RequestEstimate>,
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
            next_request: None,
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
        next_request: None,
    })
}

/// [`check_with`], then, when no limit is reached, whether the next model
/// request is estimated to pass one: such a limit is reported as reached, with
/// its [`LimitHit::next_request`] set, so the request is not made. `inputs` is
/// asked only when a limit is in force. A failed estimate is logged and the
/// check stands on the spend alone.
pub async fn check_next(
    session_manager: &SessionManager,
    session_id: &str,
    config: &UsageLimits,
    inputs: impl FnOnce() -> NextRequestInputs,
) -> Result<LimitCheck> {
    let mut found = check_with(session_manager, session_id, config).await?;
    if found.reached.is_some() {
        return Ok(found);
    }
    let Some(spend) = found.spend.as_ref() else {
        return Ok(found);
    };
    let inputs = inputs();
    match estimate_next_request(
        session_manager,
        session_id,
        &inputs,
        found.effective.needs_usd(),
    )
    .await
    {
        Ok(Some(estimate)) => {
            if let Some(hit) = projected_hit(&found.effective, spend, &estimate) {
                found.reached = Some(hit);
            }
            found.next_request = Some(estimate);
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(
            session_id,
            "could not estimate the next request; the usage limit checks the spend so far: {e:#}"
        ),
    }
    Ok(found)
}

/// The limit that holds `session_id`'s next model request at rest: reached,
/// or the next request (the previous one plus its answer,
/// [`NextRequestInputs::at_rest`]) estimated to pass it. The paused state the
/// composer and the prologue show; the prologue, the model calls a turn makes
/// at its end, `/compact` and a chat's title ask it, each before a model
/// request of its own. Nothing is stored or recorded. A failed read is logged
/// and treated as not held, like [`reached_limit`].
pub async fn hold(
    session_manager: &SessionManager,
    session_id: &str,
    inputs: NextRequestInputs,
) -> Option<LimitHit> {
    let config = UsageLimits::load();
    match check_next(session_manager, session_id, &config, || inputs).await {
        Ok(check) => check.reached,
        Err(e) => {
            tracing::warn!(
                session_id,
                "usage limit check failed, not enforced this time: {e:#}"
            );
            None
        }
    }
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

/// Why a Crew task may not start now, `None` when it may. `asking_chat` is the chat whose
/// `tasks_ask` asked for it, `None` for a task started from Crew itself: that chat is refused at
/// its own limit as well as the month's ("refuse at limit only", owner 2026-11-13), while the
/// task's spend stays on its own task chat and the month. A failed read is logged and does not
/// refuse, like [`reached_limit`].
pub async fn crew_task_refusal(
    session_manager: &SessionManager,
    asking_chat: Option<&str>,
    config: &UsageLimits,
) -> Option<&'static str> {
    if let Some(chat) = asking_chat {
        match check_with(session_manager, chat, config).await {
            Ok(found) => {
                if let Some(hit) = found.reached {
                    return Some(match hit.scope {
                        LimitScope::Month => CREW_TASK_MONTH_LIMIT,
                        LimitScope::Chat => CREW_TASK_ASK_LIMIT_REFUSAL,
                    });
                }
            }
            Err(e) => tracing::warn!(
                session_id = chat,
                "usage limit check failed, not enforced this time: {e:#}"
            ),
        }
    }
    reached_monthly_limit_with(session_manager, config)
        .await
        .map(|_| CREW_TASK_MONTH_LIMIT)
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

/// The monthly limit's state on its own, for surfaces with no chat (a new
/// chat's composer, Settings): `None` when no monthly limit is in force.
#[derive(Debug, Clone, PartialEq)]
pub struct MonthlyLimitState {
    pub spend: SpendSnapshot,
    pub reached: Option<LimitHit>,
    pub warning: Option<LimitHit>,
}

/// [`MonthlyLimitState`] against given configured limits.
pub async fn monthly_state_with(
    session_manager: &SessionManager,
    config: &UsageLimits,
) -> Result<Option<MonthlyLimitState>> {
    let effective = resolve(Some(ChatLimitOverride::Unlimited), config);
    if effective.is_unlimited() {
        return Ok(None);
    }
    // The month figures do not depend on the chat named here.
    let spend = session_manager
        .spend_snapshot("", effective.needs_usd())
        .await?;
    let (reached, warning) = evaluate(&effective, &spend);
    Ok(Some(MonthlyLimitState {
        spend,
        reached,
        warning,
    }))
}

/// Per-reply state on the agent: whether this reply already warned, whether
/// it already stopped (so a second stop point adds no second notice), whether
/// a limit cut it short, and the context manager's size estimate made before
/// each chat's previous request of this reply
/// ([`NextRequestInputs::previous_prompt_estimate`]).
#[derive(Debug, Default)]
pub struct ReplyLimitState {
    warned: AtomicBool,
    stopped: AtomicBool,
    interrupted: AtomicBool,
    prompt_estimates: Mutex<HashMap<String, u64>>,
}

impl ReplyLimitState {
    /// A reply's first request is estimated from the previous request plus
    /// its answer, as the composer and the prologue estimate it, plus what
    /// the reply adds ([`NextRequestInputs::added_estimate`]), which only the
    /// loop can see. So the loop is at least as strict as the other two: a
    /// chat they show open can still stop at its first request when the new
    /// message is large, never the other way round.
    pub fn begin_reply(&self) {
        self.warned.store(false, Ordering::SeqCst);
        self.stopped.store(false, Ordering::SeqCst);
        self.interrupted.store(false, Ordering::SeqCst);
        if let Ok(mut estimates) = self.prompt_estimates.lock() {
            estimates.clear();
        }
    }

    /// A limit stopped the reply before a model request it was about to make
    /// (the loop's gate, or a message sent to a paused chat), so its work was
    /// cut short. Not set by a turn that finished and then stored the stop
    /// notice for the next request.
    pub fn mark_interrupted(&self) {
        self.interrupted.store(true, Ordering::SeqCst);
    }

    /// Whether [`Self::mark_interrupted`] was called since the reply began.
    pub fn interrupted(&self) -> bool {
        self.interrupted.load(Ordering::SeqCst)
    }

    /// The estimate made before `session_id`'s previous request this reply.
    pub fn previous_prompt_estimate(&self, session_id: &str) -> Option<u64> {
        self.prompt_estimates.lock().ok()?.get(session_id).copied()
    }

    /// Remember the estimate of the request about to be sent.
    pub fn record_prompt_estimate(&self, session_id: &str, estimate: u64) {
        if let Ok(mut estimates) = self.prompt_estimates.lock() {
            estimates.insert(session_id.to_string(), estimate);
        }
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

    /// Crew tasks: the asking chat is refused at its own limit and the month's, a task started
    /// from Crew only at the month's, and nothing is refused below them or with no limit set.
    #[tokio::test]
    async fn a_crew_task_is_refused_only_at_a_reached_limit() {
        use crate::session::SideUsageEntry;
        let temp = tempfile::tempdir().unwrap();
        let (sm, asking) = manager_and_session(&temp).await;
        sm.apply_side_usage_event(SideUsageEntry {
            event_key: "crew-task-test:1".to_string(),
            session_id: asking.clone(),
            provider: "openai".to_string(),
            model_id: "gpt-test".to_string(),
            input_tokens: Some(600),
            output_tokens: Some(0),
        })
        .await
        .unwrap();

        let chat_full = UsageLimits {
            default_session_token_limit: Some(500),
            ..UsageLimits::default()
        };
        assert_eq!(
            crew_task_refusal(&sm, Some(&asking), &chat_full).await,
            Some(CREW_TASK_ASK_LIMIT_REFUSAL),
            "the asking chat is at its own limit"
        );
        assert_eq!(
            crew_task_refusal(&sm, None, &chat_full).await,
            None,
            "a task started from Crew has no asking chat, and its own chat has spent nothing"
        );

        // A subagent asks under its parent's limit: it is billed to the parent, so a parent at
        // its limit refuses the subagent's task too, though the subagent spent nothing.
        let subagent = sm
            .create_session(
                temp.path().to_path_buf(),
                "child".to_string(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        sm.update(&subagent.id)
            .parent_session_id(Some(asking.clone()))
            .apply()
            .await
            .unwrap();
        assert_eq!(
            crew_task_refusal(&sm, Some(&subagent.id), &chat_full).await,
            Some(CREW_TASK_ASK_LIMIT_REFUSAL),
            "a subagent asks under its parent chat's limit"
        );

        let month_full = UsageLimits {
            monthly_token_limit: Some(600),
            ..UsageLimits::default()
        };
        for asking_chat in [Some(asking.as_str()), None] {
            let refusal = crew_task_refusal(&sm, asking_chat, &month_full).await;
            assert_eq!(refusal, Some(CREW_TASK_MONTH_LIMIT), "{asking_chat:?}");
            assert!(refusal.unwrap().starts_with(MONTH_LIMIT_REACHED_PREFIX));
        }

        let below = UsageLimits {
            default_session_token_limit: Some(10_000),
            monthly_token_limit: Some(10_000),
            ..UsageLimits::default()
        };
        assert_eq!(crew_task_refusal(&sm, Some(&asking), &below).await, None);
        assert_eq!(
            crew_task_refusal(&sm, Some(&asking), &UsageLimits::default()).await,
            None
        );
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
            next_request: None,
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
            next_request: None,
        };
        let text = stop_notice(&month);
        assert!(text.starts_with(MONTH_LIMIT_REACHED_PREFIX));
        assert!(
            text.contains("the monthly limit of $250.00 for 2026-11 is used up"),
            "{text}"
        );
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

    /// The month's total sums every chat, private ones included, and only a
    /// caller with the user-action proof may read it; a monthly notice is
    /// stored in whichever chat reached the limit, so it names the limit and
    /// never the total, a percent of it or what is left.
    #[test]
    fn a_monthly_notice_never_carries_the_months_total() {
        let at = |used: f64, next_request: Option<f64>| LimitHit {
            scope: LimitScope::Month,
            source: LimitSource::None,
            amount: LimitAmount::Usd { used, limit: 250.0 },
            month: "2026-11".to_string(),
            next_request,
        };
        let tokens = LimitHit {
            amount: LimitAmount::Tokens {
                used: 61_234_567,
                limit: 66_000_000,
            },
            ..at(0.0, None)
        };
        let notices = [
            (stop_notice(&at(251.37, None)), "251.37"),
            (stop_notice(&at(246.13, Some(5.2))), "246.13"),
            (warning_notice(&at(213.37, None)), "213.37"),
            (warning_notice(&tokens), "61,234,567"),
        ];
        for (text, total) in &notices {
            assert!(
                text.starts_with(MONTH_LIMIT_REACHED_PREFIX)
                    || text.starts_with(MONTH_LIMIT_WARNING_PREFIX),
                "{text}"
            );
            assert!(!text.contains(total), "the month's total leaks: {text}");
            assert!(!text.contains("is left, so"), "what is left leaks: {text}");
            assert!(!text.contains("$3.87"), "what is left leaks: {text}");
            assert!(!text.contains('\u{2014}'), "{text}");
        }
        assert!(
            notices[0].0.contains("monthly limit of $250.00"),
            "{}",
            notices[0].0
        );
        assert!(
            notices[1]
                .0
                .contains("The next request is estimated at $5.20"),
            "{}",
            notices[1].0
        );
        assert!(
            notices[2]
                .0
                .contains("at least 80% of the monthly limit of $250.00"),
            "{}",
            notices[2].0
        );
        assert!(!notices[2].0.contains("85%"), "{}", notices[2].0);
        assert!(
            notices[3].0.contains("monthly limit of 66,000,000 tokens"),
            "{}",
            notices[3].0
        );
    }

    /// The cases `formatUsd` in the desktop runs too, so both print one
    /// amount one way.
    #[test]
    fn dollars_print_as_the_desktop_prints_them() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ui/desktop/src/utils/formatUsdCases.json");
        let cases: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("formatUsdCases.json"))
                .expect("formatUsdCases.json is JSON");
        for case in cases["single"].as_array().unwrap() {
            let amount = case[0].as_f64().unwrap();
            assert_eq!(format_usd(amount), case[1].as_str().unwrap(), "{amount}");
        }
        for case in cases["pairs"].as_array().unwrap() {
            let (a, b) = (case[0].as_f64().unwrap(), case[1].as_f64().unwrap());
            let (x, y) = format_usd_pair(a, b);
            assert_eq!(
                (x.as_str(), y.as_str()),
                (case[2].as_str().unwrap(), case[3].as_str().unwrap()),
                "{a} / {b}"
            );
        }
    }

    fn usage(input: i64, cache_read: i64, output: i64) -> RequestUsage {
        RequestUsage {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cache_read,
            cache_creation_tokens: 0,
            provider: Some("versa_azure".into()),
            model_id: Some("gpt-5.6-terra".into()),
        }
    }

    #[test]
    fn the_next_request_follows_the_previous_one_and_its_growth() {
        // 36,000-token prompt, nearly all cached, 5 tokens out.
        let previous = usage(1_000, 35_000, 5);

        // No estimate of the growth: the previous prompt plus its answer.
        let next = project_request(Some(&previous), None, &NextRequestInputs::default()).unwrap();
        assert_eq!(next.prompt_tokens(), 36_005);
        assert_eq!(next.output_tokens, 5);
        // The previous request's cache share carries over.
        assert_eq!(next.cache_read_tokens, 35_005, "{next:?}");

        // The prompt grew by half since the previous request (a tool result).
        let grown = project_request(
            Some(&previous),
            None,
            &NextRequestInputs {
                prompt_estimate: Some(60_000),
                previous_prompt_estimate: Some(40_000),
                ..NextRequestInputs::default()
            },
        )
        .unwrap();
        assert_eq!(grown.prompt_tokens(), 54_000);
        assert_eq!(grown.billed_tokens(), 54_005);

        // A chat's first request: the context manager's size, with the cache
        // share the model has been getting elsewhere.
        let first = project_request(
            None,
            Some(&previous),
            &NextRequestInputs {
                prompt_estimate: Some(10_000),
                ..NextRequestInputs::default()
            },
        )
        .unwrap();
        assert_eq!(first.prompt_tokens(), 10_000);
        assert!(first.cache_read_tokens > 9_000, "{first:?}");

        // Nothing to go on.
        assert_eq!(
            project_request(None, None, &NextRequestInputs::default()),
            None
        );
    }

    /// A reply's first request carries the new message, which the previous
    /// request did not: a 120,000-token paste counts before it is sent, as
    /// fresh input, while the previous prompt keeps its cache share. The loop's
    /// whole-request estimate is not what is added: it over-counts by design.
    #[test]
    fn a_replys_first_request_counts_the_new_message() {
        let previous = usage(1_000, 35_000, 5);
        let pasted = project_request(
            Some(&previous),
            None,
            &NextRequestInputs {
                prompt_estimate: Some(200_000),
                added_estimate: Some(120_000),
                ..NextRequestInputs::default()
            },
        )
        .unwrap();
        assert_eq!(pasted.prompt_tokens(), 156_005, "{pasted:?}");
        assert_eq!(pasted.cache_read_tokens, 35_005, "{pasted:?}");
        assert_eq!(pasted.input_tokens, 121_000, "{pasted:?}");

        // Nothing added (the composer and the prologue, which cannot see an
        // unsent message): the previous prompt plus its answer, as before.
        let resting =
            project_request(Some(&previous), None, &NextRequestInputs::default()).unwrap();
        assert_eq!(resting.prompt_tokens(), 36_005);

        // Inside a reply the growth since the previous request is the ratio
        // of two estimates, and what was added is already in it.
        let inside = project_request(
            Some(&previous),
            None,
            &NextRequestInputs {
                prompt_estimate: Some(60_000),
                previous_prompt_estimate: Some(40_000),
                added_estimate: Some(120_000),
                ..NextRequestInputs::default()
            },
        )
        .unwrap();
        assert_eq!(inside.prompt_tokens(), 54_000);

        // So against a 150,000-token limit with 50,000 used, the paste stops
        // the request the previous-request estimate alone let through.
        let limits = EffectiveLimits {
            chat_tokens: Some(150_000),
            chat_usd: None,
            source: LimitSource::Chat,
            month_tokens: None,
            month_usd: None,
        };
        let at = spend(50_000, 50_000);
        let estimate = |next: ProjectedRequest| RequestEstimate {
            tokens: next.billed_tokens() as u64,
            usd: None,
        };
        assert!(projected_hit(&limits, &at, &estimate(resting)).is_none());
        assert!(projected_hit(&limits, &at, &estimate(pasted)).is_some());
    }

    /// Each limit near its ceiling is warned about on its own: a month already
    /// warned about does not hide the chat crossing 80% of its own limit.
    #[tokio::test]
    async fn a_month_already_warned_does_not_hide_the_chats_warning() {
        let temp = tempfile::tempdir().unwrap();
        let (sm, id) = manager_and_session(&temp).await;
        let effective = EffectiveLimits {
            chat_tokens: Some(100),
            chat_usd: None,
            source: LimitSource::Chat,
            month_tokens: Some(1_000),
            month_usd: None,
        };
        let check_at = |chat: i64, month: i64| {
            let at = spend(chat, month);
            let (reached, warning) = evaluate(&effective, &at);
            LimitCheck {
                billing_session_id: id.clone(),
                chat_override: None,
                effective,
                spend: Some(at),
                reached,
                warning,
                next_request: None,
            }
        };

        // The month at 85%, warned once.
        let first = announce_warning_once(&sm, &check_at(10, 850))
            .await
            .unwrap()
            .expect("the month's warning");
        assert_eq!(first.scope, LimitScope::Month);
        assert!(announce_warning_once(&sm, &check_at(20, 860))
            .await
            .unwrap()
            .is_none());

        // Later the chat crosses 80% of its own limit.
        let chat = announce_warning_once(&sm, &check_at(85, 900))
            .await
            .unwrap()
            .expect("the chat's own warning");
        assert_eq!(chat.scope, LimitScope::Chat);
        assert!(warning_notice(&chat).starts_with(CHAT_LIMIT_WARNING_PREFIX));
        // Both are remembered now.
        assert!(announce_warning_once(&sm, &check_at(90, 950))
            .await
            .unwrap()
            .is_none());
    }

    #[test]
    fn a_request_that_would_pass_a_limit_is_a_projected_stop() {
        let limits = EffectiveLimits {
            chat_tokens: Some(90_000),
            chat_usd: Some(0.18),
            source: LimitSource::Chat,
            month_tokens: None,
            month_usd: None,
        };
        let mut at = spend(72_000, 72_000);
        at.chat_usd = Some(0.1766);
        // Under both limits now; the next request passes the token one.
        let next = RequestEstimate {
            tokens: 36_000,
            usd: Some(0.001),
        };
        let hit = projected_hit(&limits, &at, &next).expect("projected");
        assert_eq!(hit.amount.axis(), "tokens");
        assert_eq!(hit.next_request, Some(36_000.0));
        let text = stop_notice(&hit);
        assert!(text.starts_with(CHAT_LIMIT_REACHED_PREFIX), "{text}");
        assert!(
            text.contains(
                "The next request is estimated at 36,000 tokens and 18,000 tokens is left"
            ),
            "{text}"
        );

        // The dollar limit: $0.1766 used, $0.0074 more passes $0.18.
        let next = RequestEstimate {
            tokens: 1,
            usd: Some(0.0074),
        };
        let hit = projected_hit(&limits, &at, &next).expect("projected");
        let text = stop_notice(&hit);
        assert!(
            text.contains("$0.177 used, of its limit of $0.18"),
            "{text}"
        );
        assert!(
            text.contains("estimated at $0.0074 and $0.0034 is left"),
            "{text}"
        );

        // An unpriced estimate is never compared with a dollar limit.
        let unpriced = RequestEstimate {
            tokens: 1,
            usd: None,
        };
        assert_eq!(projected_hit(&limits, &at, &unpriced), None);

        // A limit smaller than one request says so.
        let small = EffectiveLimits {
            chat_usd: Some(0.005),
            chat_tokens: None,
            ..limits
        };
        let mut fresh = spend(0, 0);
        fresh.chat_usd = Some(0.0);
        let next = RequestEstimate {
            tokens: 36_000,
            usd: Some(0.0086),
        };
        let text = stop_notice(&projected_hit(&small, &fresh, &next).expect("projected"));
        assert!(
            text.contains(
                "One request is estimated at $0.0086, more than the whole limit of $0.005"
            ),
            "{text}"
        );
        assert!(!text.contains('\u{2014}'), "{text}");
    }

    #[test]
    fn notices_never_read_used_and_limit_alike_unless_they_are() {
        let hit = LimitHit {
            scope: LimitScope::Chat,
            source: LimitSource::Default,
            amount: LimitAmount::Usd {
                used: 0.0086,
                limit: 0.013,
            },
            month: "2026-11".into(),
            next_request: None,
        };
        let text = warning_notice(&hit);
        assert!(
            text.contains("$0.0086 used, of the default per-chat limit of $0.013 (66%)"),
            "{text}"
        );
        let near = LimitHit {
            amount: LimitAmount::Usd {
                used: 0.1766,
                limit: 0.18,
            },
            ..hit
        };
        assert!(
            warning_notice(&near).contains("$0.177 used"),
            "{}",
            warning_notice(&near)
        );
    }

    #[tokio::test]
    async fn the_warning_is_given_once_per_crossing_and_again_for_a_new_limit() {
        let temp = tempfile::tempdir().unwrap();
        let (sm, id) = manager_and_session(&temp).await;
        let limit = |tokens| {
            Some(ChatLimitOverride::Limit {
                max_tokens: Some(tokens),
                max_usd: None,
            })
        };
        set_for_session(&sm, &id, limit(100)).await.unwrap();
        let check_at = |used: i64, tokens: u64| {
            let effective = resolve(limit(tokens), &UsageLimits::default());
            let at = spend(used, used);
            let (reached, warning) = evaluate(&effective, &at);
            LimitCheck {
                billing_session_id: id.clone(),
                chat_override: limit(tokens),
                effective,
                spend: Some(at),
                reached,
                warning,
                next_request: None,
            }
        };

        assert!(announce_warning_once(&sm, &check_at(50, 100))
            .await
            .unwrap()
            .is_none());
        assert!(announce_warning_once(&sm, &check_at(85, 100))
            .await
            .unwrap()
            .is_some());
        // Still near: not again, not on the next reply either.
        assert!(announce_warning_once(&sm, &check_at(90, 100))
            .await
            .unwrap()
            .is_none());
        assert!(announce_warning_once(&sm, &check_at(95, 100))
            .await
            .unwrap()
            .is_none());

        // Raised: under 80% of the new limit forgets the old warning, and
        // crossing 80% of the new one warns again.
        assert!(announce_warning_once(&sm, &check_at(95, 200))
            .await
            .unwrap()
            .is_none());
        assert!(announce_warning_once(&sm, &check_at(170, 200))
            .await
            .unwrap()
            .is_some());

        // Writing the chat's limit resets the record.
        set_for_session(&sm, &id, limit(200)).await.unwrap();
        assert!(announce_warning_once(&sm, &check_at(170, 200))
            .await
            .unwrap()
            .is_some());
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
        state.record_prompt_estimate("chat", 40_000);
        assert_eq!(state.previous_prompt_estimate("chat"), Some(40_000));
        // A stop is not an interruption by itself: the end of a finished turn
        // stops too.
        assert!(!state.interrupted());
        state.mark_interrupted();
        assert!(state.interrupted());
        state.begin_reply();
        assert!(!state.interrupted());
        assert!(!state.stopped());
        assert!(state.take_warning());
        assert_eq!(
            state.previous_prompt_estimate("chat"),
            None,
            "a reply's first request is estimated like the composer's"
        );
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
