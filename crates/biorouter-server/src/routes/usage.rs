//! Queryable usage reporting: per-day / per-model token + cost breakdowns, a
//! month-to-date summary against the configured monthly limits, and the write
//! of those limits and of the default per-chat limits (`PUT /usage/limits`).
//! The limits are enforced by the reply loop; see
//! `docs/agent-loop/usage-limits.md`.
//!
//! Cost is computed server-side through the single shared
//! [`biorouter::providers::pricing::model_cost`] helper, so the report, the
//! summary gauge, and the chat cost popover can never price the same turn
//! differently. An unpriced (unknown) model yields a `null` cost — never a
//! misleading `$0`.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use axum::{Json, Router};
use biorouter::config::usage::{LimitChange, UsageLimitsUpdate};
use biorouter::config::{percent_of, Config, UsageLimits};
use biorouter::session::{UsageGroup, UsageReportRow, UsageSummary};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::state::AppState;

/// A day of usage report if `from`/`to` are omitted defaults to the last 30
/// days, ending now.
const DEFAULT_REPORT_WINDOW_SECS: i64 = 30 * 86_400;

fn require_billing_user(headers: &HeaderMap) -> Result<(), StatusCode> {
    // Billing retains anonymous totals after deletion; source ACLs cannot be
    // reconstructed from that aggregate, so only the human surface receives it.
    if biorouter_server::auth::user_action_proof(headers)
        == biorouter_server::auth::UserActionProof::Proven
    {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

/// Query for `GET /usage/report`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UsageReportQuery {
    /// Inclusive start of the window, unix seconds. Defaults to 30 days ago.
    pub from: Option<i64>,
    /// Inclusive end of the window, unix seconds. Defaults to now.
    pub to: Option<i64>,
    /// `day` (default), `model`, or `day_model`.
    pub group: Option<UsageGroup>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageReportResponse {
    /// Echoed resolved window bounds (unix seconds), so the client can render
    /// the exact range it got even when it sent no `from`/`to`.
    pub from: i64,
    pub to: i64,
    pub group: UsageGroup,
    pub rows: Vec<UsageReportRow>,
}

#[utoipa::path(
    get,
    path = "/usage/report",
    params(
        ("from" = Option<i64>, Query, description = "Inclusive window start, unix seconds (default: 30 days ago)"),
        ("to" = Option<i64>, Query, description = "Inclusive window end, unix seconds (default: now)"),
        ("group" = Option<UsageGroup>, Query, description = "Bucketing: day (default), model, or day_model")
    ),
    responses(
        (status = 200, description = "Usage report", body = UsageReportResponse),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Human user-action proof required for aggregate billing data"),
        (status = 500, description = "Internal server error")
    ),
    security(("api_key" = [])),
    tag = "Usage"
)]
pub async fn get_usage_report(
    State(state): State<Arc<AppState>>,
    Query(query): Query<UsageReportQuery>,
    headers: HeaderMap,
) -> Result<Json<UsageReportResponse>, StatusCode> {
    require_billing_user(&headers)?;
    let now = chrono::Utc::now().timestamp();
    let to = query.to.unwrap_or(now);
    let from = query.from.unwrap_or(to - DEFAULT_REPORT_WINDOW_SECS);
    let group = query.group.unwrap_or(UsageGroup::Day);

    let rows = state
        .session_manager()
        .get_usage_report(from, to, group)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(UsageReportResponse {
        from,
        to,
        group,
        rows,
    }))
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummaryResponse {
    #[serde(flatten)]
    pub summary: UsageSummary,
    /// Configured monthly token allowance, or `null` when unset.
    pub monthly_token_limit: Option<i64>,
    /// Configured monthly dollar budget, or `null` when unset.
    pub monthly_dollar_limit: Option<f64>,
    /// MTD billed tokens as a percent of the token limit; `null` when no limit
    /// is set or the historical billed total is incomplete.
    pub token_percent: Option<f64>,
    /// MTD cost as a percent of the dollar limit; `null` when no limit is set or
    /// the MTD cost is unknown or only a lower bound.
    pub dollar_percent: Option<f64>,
    /// Default per-chat token limit for chats without their own, or `null`.
    pub default_session_token_limit: Option<i64>,
    /// Default per-chat dollar limit for chats without their own, or `null`.
    pub default_session_dollar_limit: Option<f64>,
}

/// Assemble the summary response from the raw totals + configured limits. Pure,
/// so the percent math is unit-tested without a database or config file.
fn build_summary_response(summary: UsageSummary, limits: UsageLimits) -> UsageSummaryResponse {
    // The limits in force: a zero or negative value in config is unlimited.
    let monthly_token_limit = limits.monthly_tokens().map(|limit| limit as i64);
    let monthly_dollar_limit = limits.monthly_usd();
    let token_percent = summary.month_to_date.total_tokens.and_then(|tokens| {
        percent_of(tokens as f64, monthly_token_limit.map(|limit| limit as f64))
    });
    let dollar_percent =
        if summary.month_to_date.has_unpriced || summary.month_to_date.cost_excludes_cache {
            None
        } else {
            summary
                .month_to_date
                .cost
                .and_then(|cost| percent_of(cost, monthly_dollar_limit))
        };

    UsageSummaryResponse {
        summary,
        monthly_token_limit,
        monthly_dollar_limit,
        token_percent,
        dollar_percent,
        default_session_token_limit: limits.default_session_tokens().map(|limit| limit as i64),
        default_session_dollar_limit: limits.default_session_usd(),
    }
}

#[utoipa::path(
    get,
    path = "/usage/summary",
    responses(
        (status = 200, description = "Month-to-date usage vs the configured budget", body = UsageSummaryResponse),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Human user-action proof required for aggregate billing data"),
        (status = 500, description = "Internal server error")
    ),
    security(("api_key" = [])),
    tag = "Usage"
)]
pub async fn get_usage_summary(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<UsageSummaryResponse>, StatusCode> {
    require_billing_user(&headers)?;
    let summary = state
        .session_manager()
        .get_usage_summary()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(build_summary_response(summary, UsageLimits::load())))
}

/// A field that may be absent (keep), `null` (unlimited) or a value (set).
fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Body of `PUT /usage/limits`. A field left out keeps what is stored, `null`
/// makes that limit unlimited, and a value above 0 sets it.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateUsageLimitsRequest {
    /// Billed tokens per local calendar month, over every chat.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<i64>, nullable = true)]
    pub monthly_token_limit: Option<Option<i64>>,
    /// Priced dollars per local calendar month, over every chat.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<f64>, nullable = true)]
    pub monthly_dollar_limit: Option<Option<f64>>,
    /// Billed tokens per chat, for chats without their own limit.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<i64>, nullable = true)]
    pub default_session_token_limit: Option<Option<i64>>,
    /// Priced dollars per chat, for chats without their own limit.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<f64>, nullable = true)]
    pub default_session_dollar_limit: Option<Option<f64>>,
}

/// The configured limits, as stored after a write; `null` is unlimited.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageLimitsResponse {
    pub monthly_token_limit: Option<i64>,
    pub monthly_dollar_limit: Option<f64>,
    pub default_session_token_limit: Option<i64>,
    pub default_session_dollar_limit: Option<f64>,
}

impl From<&UsageLimits> for UsageLimitsResponse {
    fn from(limits: &UsageLimits) -> Self {
        Self {
            monthly_token_limit: limits.monthly_tokens().map(|limit| limit as i64),
            monthly_dollar_limit: limits.monthly_usd(),
            default_session_token_limit: limits.default_session_tokens().map(|limit| limit as i64),
            default_session_dollar_limit: limits.default_session_usd(),
        }
    }
}

fn token_change(name: &str, value: Option<Option<i64>>) -> Result<LimitChange<i64>, String> {
    match value {
        None => Ok(LimitChange::Keep),
        Some(None) => Ok(LimitChange::Clear),
        Some(Some(tokens)) if tokens > 0 => Ok(LimitChange::Set(tokens)),
        Some(Some(_)) => Err(format!(
            "`{name}` must be a whole number above 0, or null for unlimited."
        )),
    }
}

fn dollar_change(name: &str, value: Option<Option<f64>>) -> Result<LimitChange<f64>, String> {
    match value {
        None => Ok(LimitChange::Keep),
        Some(None) => Ok(LimitChange::Clear),
        Some(Some(usd)) if usd.is_finite() && usd > 0.0 => Ok(LimitChange::Set(usd)),
        Some(Some(_)) => Err(format!(
            "`{name}` must be a number above 0, or null for unlimited."
        )),
    }
}

/// The update a request asks for, or the sentence that says why it is invalid.
fn requested_update(request: UpdateUsageLimitsRequest) -> Result<UsageLimitsUpdate, String> {
    Ok(UsageLimitsUpdate {
        monthly_token_limit: token_change("monthlyTokenLimit", request.monthly_token_limit)?,
        monthly_dollar_limit: dollar_change("monthlyDollarLimit", request.monthly_dollar_limit)?,
        default_session_token_limit: token_change(
            "defaultSessionTokenLimit",
            request.default_session_token_limit,
        )?,
        default_session_dollar_limit: dollar_change(
            "defaultSessionDollarLimit",
            request.default_session_dollar_limit,
        )?,
    })
}

#[utoipa::path(
    put,
    path = "/usage/limits",
    request_body = UpdateUsageLimitsRequest,
    responses(
        (status = 200, description = "The monthly and default per-chat limits as stored", body = UsageLimitsResponse),
        (status = 400, description = "A body that could not be read or a limit that is not above 0 (plain text)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Human user-action proof required: a model may not raise or lift a limit"),
        (status = 500, description = "Internal server error")
    ),
    security(("api_key" = [])),
    tag = "Usage"
)]
pub async fn put_usage_limits(headers: HeaderMap, body: Bytes) -> Response {
    if let Err(status) = require_billing_user(&headers) {
        return status.into_response();
    }
    let request: UpdateUsageLimitsRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("The usage limits could not be read: {error}"),
            )
                .into_response()
        }
    };
    let update = match requested_update(request) {
        Ok(update) => update,
        Err(sentence) => return (StatusCode::BAD_REQUEST, sentence).into_response(),
    };
    match UsageLimits::apply_update(Config::global(), &update) {
        Ok(stored) => Json(UsageLimitsResponse::from(&stored)).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/usage/report", get(get_usage_report))
        .route("/usage/summary", get(get_usage_summary))
        .route("/usage/limits", put(put_usage_limits))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter::session::{UsageReportRow, UsageTotals};

    fn totals(total_tokens: i64, cost: Option<f64>, has_unpriced: bool) -> UsageTotals {
        UsageTotals {
            input_tokens: total_tokens,
            output_tokens: 0,
            total_tokens: Some(total_tokens),
            cache_read_tokens: Some(0),
            cache_creation_tokens: Some(0),
            turns: 1,
            cost,
            has_unpriced,
            cost_excludes_cache: false,
            decision_requests: 0,
            decisions: None,
        }
    }

    fn summary_with_mtd(mtd: UsageTotals) -> UsageSummary {
        UsageSummary {
            month: "2026-07".to_string(),
            month_to_date: mtd,
            all_time: totals(0, None, false),
        }
    }

    #[test]
    fn percent_is_computed_when_both_limits_are_set() {
        let summary = summary_with_mtd(totals(33_000_000, Some(125.0), false));
        let limits = UsageLimits {
            monthly_token_limit: Some(66_000_000),
            monthly_dollar_limit: Some(250.0),
            ..UsageLimits::default()
        };
        let resp = build_summary_response(summary, limits);
        // 33M / 66M = 50%, $125 / $250 = 50%.
        assert_eq!(resp.token_percent, Some(50.0));
        assert_eq!(resp.dollar_percent, Some(50.0));
        assert_eq!(resp.monthly_token_limit, Some(66_000_000));
        assert_eq!(resp.monthly_dollar_limit, Some(250.0));
    }

    #[test]
    fn percents_are_null_when_no_limit_configured() {
        let summary = summary_with_mtd(totals(1_000, Some(1.0), false));
        let resp = build_summary_response(summary, UsageLimits::default());
        assert_eq!(resp.token_percent, None);
        assert_eq!(resp.dollar_percent, None);
        assert_eq!(resp.monthly_token_limit, None);
        assert_eq!(resp.monthly_dollar_limit, None);
    }

    #[test]
    fn dollar_percent_is_null_when_mtd_cost_is_unknown() {
        // A dollar limit is set, but MTD cost is null (every model unpriced) — we
        // must not fabricate a percent from a missing cost.
        let summary = summary_with_mtd(totals(500, None, true));
        let limits = UsageLimits {
            monthly_token_limit: Some(1_000),
            monthly_dollar_limit: Some(100.0),
            ..UsageLimits::default()
        };
        let resp = build_summary_response(summary, limits);
        assert_eq!(resp.token_percent, Some(50.0), "tokens still measurable");
        assert_eq!(resp.dollar_percent, None, "no cost, no dollar percent");
    }

    #[test]
    fn dollar_percent_is_null_when_cost_is_only_a_known_subtotal() {
        let summary = summary_with_mtd(totals(500, Some(25.0), true));
        let resp = build_summary_response(
            summary,
            UsageLimits {
                monthly_token_limit: None,
                monthly_dollar_limit: Some(100.0),
                ..UsageLimits::default()
            },
        );
        assert_eq!(
            resp.dollar_percent, None,
            "partial cost is not a budget percent"
        );

        let mut cache_partial = totals(500, Some(25.0), false);
        cache_partial.cost_excludes_cache = true;
        let resp = build_summary_response(
            summary_with_mtd(cache_partial),
            UsageLimits {
                monthly_token_limit: None,
                monthly_dollar_limit: Some(100.0),
                ..UsageLimits::default()
            },
        );
        assert_eq!(resp.dollar_percent, None);
    }

    #[test]
    fn summary_response_flattens_to_camelcase() {
        let summary = summary_with_mtd(totals(1_000, Some(2.5), false));
        let resp = build_summary_response(
            summary,
            UsageLimits {
                monthly_token_limit: Some(10_000),
                monthly_dollar_limit: None,
                ..UsageLimits::default()
            },
        );
        let json = serde_json::to_value(&resp).unwrap();
        // Flattened summary fields.
        assert_eq!(json.get("month").and_then(|v| v.as_str()), Some("2026-07"));
        assert!(json.get("monthToDate").is_some(), "got {json}");
        assert!(json.get("allTime").is_some());
        // Limit fields.
        assert_eq!(
            json.get("monthlyTokenLimit").and_then(|v| v.as_i64()),
            Some(10_000)
        );
        assert!(
            json.get("monthlyDollarLimit").unwrap().is_null(),
            "unset dollar limit is null"
        );
        assert_eq!(
            json.get("tokenPercent").and_then(|v| v.as_f64()),
            Some(10.0)
        );
        assert!(json.get("dollarPercent").unwrap().is_null());
    }

    #[test]
    fn summary_carries_the_default_session_limits_and_drops_non_positive_ones() {
        let summary = summary_with_mtd(totals(1_000, Some(2.5), false));
        let resp = build_summary_response(
            summary,
            UsageLimits {
                monthly_token_limit: Some(0),
                monthly_dollar_limit: None,
                default_session_token_limit: Some(5_000),
                default_session_dollar_limit: Some(-1.0),
            },
        );
        let json = serde_json::to_value(&resp).unwrap();
        assert!(
            json["monthlyTokenLimit"].is_null(),
            "0 is unlimited: {json}"
        );
        assert!(json["tokenPercent"].is_null());
        assert_eq!(json["defaultSessionTokenLimit"], 5_000);
        assert!(json["defaultSessionDollarLimit"].is_null());
    }

    #[test]
    fn a_limits_body_keeps_clears_or_sets_each_field() {
        let request: UpdateUsageLimitsRequest = serde_json::from_value(serde_json::json!({
            "monthlyTokenLimit": 100,
            "monthlyDollarLimit": null,
            "defaultSessionDollarLimit": 2.5
        }))
        .unwrap();
        let update = requested_update(request).unwrap();
        assert_eq!(update.monthly_token_limit, LimitChange::Set(100));
        assert_eq!(update.monthly_dollar_limit, LimitChange::Clear);
        assert_eq!(update.default_session_token_limit, LimitChange::Keep);
        assert_eq!(update.default_session_dollar_limit, LimitChange::Set(2.5));

        for bad in [
            serde_json::json!({ "monthlyTokenLimit": 0 }),
            serde_json::json!({ "monthlyTokenLimit": -3 }),
            serde_json::json!({ "defaultSessionDollarLimit": 0.0 }),
        ] {
            let request: UpdateUsageLimitsRequest = serde_json::from_value(bad.clone()).unwrap();
            assert!(requested_update(request).is_err(), "{bad}");
        }
        assert!(serde_json::from_value::<UpdateUsageLimitsRequest>(
            serde_json::json!({ "weeklyTokenLimit": 1 })
        )
        .is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial_test::serial]
    async fn only_a_person_writes_the_limits_and_the_write_merges() {
        use crate::routes::session::diverge_tests::{
            install_test_user_action_key, TEST_USER_ACTION_KEY,
        };
        use axum::body::{to_bytes, Body};
        use axum::http::Request;
        use tower::ServiceExt;

        install_test_user_action_key();
        let put = |body: serde_json::Value, proven: bool| async move {
            let mut builder = Request::builder()
                .method("PUT")
                .uri("/usage/limits")
                .header("content-type", "application/json");
            if proven {
                builder = builder.header("X-User-Action", TEST_USER_ACTION_KEY);
            }
            let state = AppState::new().await.unwrap();
            let res = routes(state)
                .oneshot(builder.body(Body::from(body.to_string())).unwrap())
                .await
                .unwrap();
            let status = res.status();
            let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
            (status, String::from_utf8_lossy(&bytes).into_owned())
        };

        let (status, _) = put(serde_json::json!({ "monthlyTokenLimit": 1 }), false).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "a model cannot set a limit");

        // Limits far out of reach, so a parallel test is never stopped by them.
        let (status, body) = put(
            serde_json::json!({
                "monthlyTokenLimit": 900_000_000_000_000i64,
                "defaultSessionTokenLimit": 800_000_000_000_000i64
            }),
            true,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let stored: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(stored["monthlyTokenLimit"], 900_000_000_000_000i64);
        assert_eq!(stored["defaultSessionTokenLimit"], 800_000_000_000_000i64);

        // A field left out is kept.
        let (status, body) = put(serde_json::json!({ "monthlyTokenLimit": null }), true).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let stored: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(stored["monthlyTokenLimit"].is_null(), "{body}");
        assert_eq!(stored["defaultSessionTokenLimit"], 800_000_000_000_000i64);

        let (status, _) = put(serde_json::json!({ "monthlyTokenLimit": -1 }), true).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, body) = put(
            serde_json::json!({ "defaultSessionTokenLimit": null }),
            true,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    #[test]
    fn report_response_serializes_rows_camelcase_with_null_cost() {
        let resp = UsageReportResponse {
            from: 100,
            to: 200,
            group: UsageGroup::Day,
            rows: vec![
                UsageReportRow {
                    date: Some("2026-07-10".to_string()),
                    model_id: None,
                    provider: None,
                    input_tokens: 10,
                    output_tokens: 5,
                    total_tokens: Some(15),
                    cache_read_tokens: Some(0),
                    cache_creation_tokens: Some(0),
                    turns: 1,
                    cost: Some(1.25),
                    has_unpriced: true,
                    cost_excludes_cache: false,
                    decision_requests: 0,
                    decisions: None,
                },
                UsageReportRow {
                    date: Some("2026-07-11".to_string()),
                    model_id: Some("glm-5.2".to_string()),
                    provider: Some("zai".to_string()),
                    input_tokens: 20,
                    output_tokens: 10,
                    total_tokens: Some(30),
                    cache_read_tokens: Some(900),
                    cache_creation_tokens: Some(0),
                    turns: 2,
                    cost: None,
                    has_unpriced: true,
                    cost_excludes_cache: false,
                    decision_requests: 0,
                    decisions: None,
                },
            ],
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json.get("group").and_then(|v| v.as_str()), Some("day"));
        let rows = json.get("rows").and_then(|r| r.as_array()).unwrap();
        assert_eq!(rows[0].get("cost").and_then(|v| v.as_f64()), Some(1.25));
        assert_eq!(
            rows[0].get("hasUnpriced").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            rows[0].get("totalTokens").and_then(|v| v.as_i64()),
            Some(15)
        );
        // A null-cost row serializes cost as JSON null, never 0.
        assert!(rows[1].get("cost").unwrap().is_null());
        assert_eq!(
            rows[1].get("modelId").and_then(|v| v.as_str()),
            Some("glm-5.2")
        );
        // Cache buckets are surfaced camelCased on each row.
        assert_eq!(
            rows[1].get("cacheReadTokens").and_then(|v| v.as_i64()),
            Some(900)
        );
        assert_eq!(
            rows[0].get("cacheCreationTokens").and_then(|v| v.as_i64()),
            Some(0)
        );
    }

    /// The decision-model fields reach the wire camelCased, with a null
    /// subtotal on a bucket that has no decision requests.
    #[test]
    fn report_rows_carry_the_decision_subtotal() {
        let mut row = UsageReportRow {
            date: Some("2026-11-04".to_string()),
            model_id: None,
            provider: None,
            input_tokens: 1_200_000,
            output_tokens: 600,
            total_tokens: Some(1_200_600),
            cache_read_tokens: Some(0),
            cache_creation_tokens: Some(0),
            turns: 0,
            cost: Some(0.12),
            has_unpriced: false,
            cost_excludes_cache: false,
            decision_requests: 600,
            decisions: Some(biorouter::session::session_manager::DecisionUsageSubtotal {
                input_tokens: 1_200_000,
                output_tokens: 600,
                total_tokens: Some(1_200_600),
                cost: Some(0.12),
                has_unpriced: false,
                has_unmetered: true,
            }),
        };
        let json = serde_json::to_value(&row).unwrap();
        assert_eq!(json["decisionRequests"], 600);
        assert_eq!(json["decisions"]["totalTokens"], 1_200_600);
        assert_eq!(json["decisions"]["hasUnpriced"], false);
        assert_eq!(json["decisions"]["hasUnmetered"], true);
        row.decisions = None;
        assert!(serde_json::to_value(&row).unwrap()["decisions"].is_null());
    }
}
