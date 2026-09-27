//! Queryable usage reporting: per-day / per-model token + cost breakdowns and a
//! month-to-date summary against the (display-only) configured monthly budget.
//!
//! Cost is computed server-side through the single shared
//! [`biorouter::providers::pricing::model_cost`] helper, so the report, the
//! summary gauge, and the chat cost popover can never price the same turn
//! differently. An unpriced (unknown) model yields a `null` cost — never a
//! misleading `$0`.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use biorouter::config::{percent_of, UsageLimits};
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
}

/// Assemble the summary response from the raw totals + configured limits. Pure,
/// so the percent math is unit-tested without a database or config file.
fn build_summary_response(summary: UsageSummary, limits: UsageLimits) -> UsageSummaryResponse {
    let token_percent = summary.month_to_date.total_tokens.and_then(|tokens| {
        percent_of(
            tokens as f64,
            limits.monthly_token_limit.map(|limit| limit as f64),
        )
    });
    let dollar_percent =
        if summary.month_to_date.has_unpriced || summary.month_to_date.cost_excludes_cache {
            None
        } else {
            summary
                .month_to_date
                .cost
                .and_then(|cost| percent_of(cost, limits.monthly_dollar_limit))
        };

    UsageSummaryResponse {
        summary,
        monthly_token_limit: limits.monthly_token_limit,
        monthly_dollar_limit: limits.monthly_dollar_limit,
        token_percent,
        dollar_percent,
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

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/usage/report", get(get_usage_report))
        .route("/usage/summary", get(get_usage_summary))
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
}
