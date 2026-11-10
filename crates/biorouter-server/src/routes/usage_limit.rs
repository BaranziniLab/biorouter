//! A chat's usage limit: `GET|PUT /sessions/{session_id}/usage-limit`.
//!
//! | Route | Gate |
//! |---|---|
//! | `GET /sessions/{id}/usage-limit` | [`session_reach`](crate::routes::session_reach::session_reach) |
//! | `PUT /sessions/{id}/usage-limit` | `session_reach`, then the user-action proof on a keyed daemon (so a model cannot lift its own limit), then the subagent rule |
//!
//! The answer is the chat's effective limit (its own, the default per-chat
//! limit, or none), its override, the configured defaults and monthly limit,
//! what the chat (with its subagents) and the month have used, and whether a
//! limit is reached or near. A subagent is held to its parent chat's limit, so
//! a read names the parent's figures and a write to a subagent is refused.
//! Writing a chat's limit is a deliberate change to the chat and stamps its
//! activity (coordinator ruling 16). See `docs/agent-loop/usage-limits.md`.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use biorouter::agents::usage_limit::{self, ChatLimitOverride, LimitScope, LimitSource};
use biorouter::config::UsageLimits;
use biorouter::session::{Session, SessionType};
use biorouter_server::auth::{user_action_proof, UserActionProof};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::state::AppState;

/// The refusal of a usage-limit write that carried no proof of a person, on a
/// daemon that holds a user-action key.
pub const USAGE_LIMIT_NEED_USER: &str = "Changing a chat's usage limit is the user's decision: \
     the limit is what stops this chat spending, and this request carried no proof that it came \
     from the person at the keyboard. Nothing was changed. Do not retry; the same call will be \
     refused again. If the work needs a higher limit, stop and ask the user to raise or lift it \
     from the cost at the lower right of the composer.";

/// …and on a desktop daemon that was meant to receive a key and did not.
pub const USAGE_LIMIT_KEY_MISSING: &str = crate::routes::decisions::DECISION_SETTINGS_KEY_MISSING;

/// A write aimed at a subagent's chat, which is held to its parent's limit.
pub const USAGE_LIMIT_SUBAGENT: &str = "A subagent is held to the usage limit of the chat that \
     started it. Set the limit on that chat instead. Nothing was changed.";

/// Where a chat's effective limit came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum UsageLimitSource {
    Chat,
    Default,
    None,
}

impl From<LimitSource> for UsageLimitSource {
    fn from(source: LimitSource) -> Self {
        match source {
            LimitSource::Chat => UsageLimitSource::Chat,
            LimitSource::Default => UsageLimitSource::Default,
            LimitSource::None => UsageLimitSource::None,
        }
    }
}

/// Which limit is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum UsageLimitReason {
    Chat,
    Month,
}

/// A pair of limits; `null` is unlimited on that axis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageLimitAmounts {
    pub max_tokens: Option<u64>,
    pub max_usd: Option<f64>,
}

/// The per-chat limit in force for this chat.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveChatLimit {
    pub max_tokens: Option<u64>,
    pub max_usd: Option<f64>,
    pub source: UsageLimitSource,
}

/// The chat's own override, when it has one.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatUsageLimitOverride {
    /// `unlimited` or `limit`.
    pub mode: UsageLimitMode,
    pub max_tokens: Option<u64>,
    pub max_usd: Option<f64>,
}

/// A write's mode: return to the default, lift every per-chat limit for this
/// chat, or set this chat's own limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum UsageLimitMode {
    Inherit,
    Unlimited,
    Limit,
}

/// What the chat (with its subagents) and the month have used.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageLimitUsed {
    pub chat_tokens: i64,
    /// `null` when no dollar limit is in force or nothing is priced.
    pub chat_usd: Option<f64>,
    pub chat_usd_complete: bool,
    pub month_tokens: i64,
    pub month_usd: Option<f64>,
    pub month_usd_complete: bool,
    /// The local month the month figures cover, `YYYY-MM`.
    pub month: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionUsageLimitResponse {
    pub limit: EffectiveChatLimit,
    /// `null` when the chat inherits the default.
    pub chat_override: Option<ChatUsageLimitOverride>,
    /// The default per-chat limit in Settings.
    pub defaults: UsageLimitAmounts,
    pub monthly: UsageLimitAmounts,
    pub used: UsageLimitUsed,
    /// A limit is reached: the chat makes no model request until it is raised
    /// or lifted (or, for the month, the next local month begins).
    pub paused: bool,
    pub reason: Option<UsageLimitReason>,
    /// A limit is at 80% or more and none is reached.
    pub warn: bool,
    /// A per-chat dollar limit is set but nothing the chat used is priced, so
    /// it cannot fire.
    pub chat_dollar_limit_inert: bool,
    /// A monthly dollar limit is set but nothing this month is priced, so it
    /// cannot fire.
    pub month_dollar_limit_inert: bool,
    /// The chat whose limit and spend this is: the chat itself, or a
    /// subagent's parent.
    pub billing_session_id: String,
}

/// Body of `PUT /sessions/{session_id}/usage-limit`.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetSessionUsageLimitRequest {
    pub mode: UsageLimitMode,
    /// With `limit`: billed tokens this chat may use, a whole number above 0.
    #[serde(default)]
    pub max_tokens: Option<u64>,
    /// With `limit`: priced dollars this chat may use, above 0.
    #[serde(default)]
    pub max_usd: Option<f64>,
}

fn plain(status: StatusCode, message: impl Into<String>) -> Response {
    (status, message.into()).into_response()
}

/// The stored override a request asks for (`None` is inherit), or the sentence
/// that says why the body is invalid.
fn requested_override(
    request: &SetSessionUsageLimitRequest,
) -> Result<Option<ChatLimitOverride>, &'static str> {
    match request.mode {
        UsageLimitMode::Inherit | UsageLimitMode::Unlimited
            if request.max_tokens.is_some() || request.max_usd.is_some() =>
        {
            Err("`maxTokens` and `maxUsd` go with `mode: limit` only.")
        }
        UsageLimitMode::Inherit => Ok(None),
        UsageLimitMode::Unlimited => Ok(Some(ChatLimitOverride::Unlimited)),
        UsageLimitMode::Limit => {
            if request.max_tokens.is_none() && request.max_usd.is_none() {
                return Err("`mode: limit` needs `maxTokens`, `maxUsd` or both.");
            }
            if request.max_tokens == Some(0) {
                return Err("`maxTokens` must be a whole number above 0.");
            }
            if request
                .max_usd
                .is_some_and(|usd| !usd.is_finite() || usd <= 0.0)
            {
                return Err("`maxUsd` must be a number above 0.");
            }
            Ok(Some(ChatLimitOverride::Limit {
                max_tokens: request.max_tokens,
                max_usd: request.max_usd,
            }))
        }
    }
}

fn override_wire(value: ChatLimitOverride) -> ChatUsageLimitOverride {
    match value {
        ChatLimitOverride::Unlimited => ChatUsageLimitOverride {
            mode: UsageLimitMode::Unlimited,
            max_tokens: None,
            max_usd: None,
        },
        ChatLimitOverride::Limit {
            max_tokens,
            max_usd,
        } => ChatUsageLimitOverride {
            mode: UsageLimitMode::Limit,
            max_tokens,
            max_usd,
        },
    }
}

/// The id's shape, then the reach gate (its refusal word for word), then
/// whether the chat exists, in that order, as `routes::decisions` does: only an
/// admitted caller learns that a chat is absent.
async fn reach_chat(
    state: &AppState,
    session_id: &str,
    headers: &HeaderMap,
) -> Result<Session, Response> {
    if !crate::routes::session::is_valid_session_id(session_id) {
        return Err(plain(
            StatusCode::BAD_REQUEST,
            "That is not a chat id: it must be 1 to 128 letters, digits, `-` or `_`.",
        ));
    }
    let manager = state.session_manager();
    crate::routes::session_reach::session_reach(manager, session_id, headers)
        .await
        .map_err(IntoResponse::into_response)?;
    match manager.get_session(session_id, false).await {
        Ok(session) => Ok(session),
        Err(error) if error.to_string().contains("not found") => Err(plain(
            StatusCode::NOT_FOUND,
            "There is no chat with that id.",
        )),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    }
}

/// The proof rule of a write, pure so every corner is tested: refused without
/// the proof on a keyed daemon, refused on a desktop daemon whose key never
/// arrived, admitted on a daemon that can never check one (`biorouter serve`).
fn proof_refusal(proof: UserActionProof, expected_key: bool) -> Option<&'static str> {
    match proof {
        UserActionProof::Proven => None,
        UserActionProof::Unproven => Some(USAGE_LIMIT_NEED_USER),
        UserActionProof::NoKeyInstalled if expected_key => Some(USAGE_LIMIT_KEY_MISSING),
        UserActionProof::NoKeyInstalled => None,
    }
}

/// Every refusal a write meets after the reach gate: the proof, then the
/// subagent rule (SD-11's sentence on a keyless daemon, else the plain 400).
fn write_refusal(session: &Session, headers: &HeaderMap) -> Option<Response> {
    let proof = user_action_proof(headers);
    if let Some(sentence) = proof_refusal(
        proof,
        biorouter_server::launch::expected_a_user_action_key(),
    ) {
        return Some(plain(StatusCode::FORBIDDEN, sentence));
    }
    if session.session_type == SessionType::SubAgent {
        return Some(if proof == UserActionProof::NoKeyInstalled {
            plain(
                StatusCode::FORBIDDEN,
                crate::routes::agent::SUBAGENT_CONTROL_NO_KEY,
            )
        } else {
            plain(StatusCode::BAD_REQUEST, USAGE_LIMIT_SUBAGENT)
        });
    }
    None
}

/// The response for `session_id` as it stands now.
pub(crate) async fn usage_limit_state(
    state: &AppState,
    session_id: &str,
    config: &UsageLimits,
) -> anyhow::Result<SessionUsageLimitResponse> {
    let manager = state.session_manager();
    let billing = usage_limit::billing_session_id(manager, session_id).await;
    let chat_override = usage_limit::for_session_lenient(manager, &billing).await;
    let effective = usage_limit::resolve(chat_override, config);
    let spend = manager
        .spend_snapshot(&billing, effective.needs_usd())
        .await?;
    let (reached, warning) = usage_limit::evaluate(&effective, &spend);
    Ok(SessionUsageLimitResponse {
        limit: EffectiveChatLimit {
            max_tokens: effective.chat_tokens,
            max_usd: effective.chat_usd,
            source: effective.source.into(),
        },
        chat_override: chat_override.map(|value| override_wire(value.normalized())),
        defaults: UsageLimitAmounts {
            max_tokens: config.default_session_tokens(),
            max_usd: config.default_session_usd(),
        },
        monthly: UsageLimitAmounts {
            max_tokens: config.monthly_tokens(),
            max_usd: config.monthly_usd(),
        },
        paused: reached.is_some(),
        reason: reached.as_ref().map(|hit| match hit.scope {
            LimitScope::Chat => UsageLimitReason::Chat,
            LimitScope::Month => UsageLimitReason::Month,
        }),
        warn: warning.is_some(),
        chat_dollar_limit_inert: effective.chat_usd.is_some() && spend.chat_usd.is_none(),
        month_dollar_limit_inert: effective.month_usd.is_some() && spend.month_usd.is_none(),
        used: UsageLimitUsed {
            chat_tokens: spend.chat_tokens,
            chat_usd: spend.chat_usd,
            chat_usd_complete: spend.chat_usd_complete,
            month_tokens: spend.month_tokens,
            month_usd: spend.month_usd,
            month_usd_complete: spend.month_usd_complete,
            month: spend.month,
        },
        billing_session_id: billing,
    })
}

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/usage-limit",
    tag = "Usage",
    params(
        ("session_id" = String, Path, description = "The chat whose usage limit to read")
    ),
    responses(
        (status = 200, description = "The chat's effective limit, its override, the defaults, \
                                      the monthly limit, what was used and whether a limit is \
                                      reached", body = SessionUsageLimitResponse),
        (status = 400, description = "Invalid session id (body = plain text)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused by a privacy boundary: the same refusal, word for \
                                      word, that `GET /sessions/{session_id}` gives (body = plain \
                                      text)"),
        (status = 404, description = "Session not found (body = plain text)"),
        (status = 500, description = "Internal server error")
    ),
    security(("api_key" = []))
)]
pub async fn get_session_usage_limit(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(refusal) = reach_chat(&state, &session_id, &headers).await {
        return refusal;
    }
    match usage_limit_state(&state, &session_id, &UsageLimits::load()).await {
        Ok(body) => Json(body).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[utoipa::path(
    put,
    path = "/sessions/{session_id}/usage-limit",
    tag = "Usage",
    params(
        ("session_id" = String, Path, description = "The chat whose usage limit to set")
    ),
    request_body = SetSessionUsageLimitRequest,
    responses(
        (status = 200, description = "The chat's usage limit as stored, in the shape `GET` \
                                      answers", body = SessionUsageLimitResponse),
        (status = 400, description = "Invalid session id, a body that could not be read or is \
                                      invalid, or a subagent's chat; the body is the sentence \
                                      that says why (plain text)"),
        (status = 401, description = "Unauthorized - Invalid or missing API key"),
        (status = 403, description = "Refused: the chat is out of this caller's reach, or, on a \
                                      daemon that holds a user-action key, the request carried \
                                      no proof it came from the person at the keyboard, or, on \
                                      a daemon that holds none, the chat is a subagent's (body \
                                      = plain text)"),
        (status = 404, description = "Session not found (body = plain text)"),
        (status = 500, description = "Internal server error")
    ),
    security(("api_key" = []))
)]
pub async fn put_session_usage_limit(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let session = match reach_chat(&state, &session_id, &headers).await {
        Ok(session) => session,
        Err(refusal) => return refusal,
    };
    if let Some(refusal) = write_refusal(&session, &headers) {
        return refusal;
    }
    let request: SetSessionUsageLimitRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => {
            return plain(
                StatusCode::BAD_REQUEST,
                format!("The usage limit could not be read: {error}"),
            )
        }
    };
    let value = match requested_override(&request) {
        Ok(value) => value,
        Err(sentence) => return plain(StatusCode::BAD_REQUEST, sentence),
    };
    match usage_limit::set_for_session(state.session_manager(), &session_id, value).await {
        Ok(true) => {}
        Ok(false) => return plain(StatusCode::NOT_FOUND, "There is no chat with that id."),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
    match usage_limit_state(&state, &session_id, &UsageLimits::load()).await {
        Ok(body) => Json(body).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/sessions/{session_id}/usage-limit",
            get(get_session_usage_limit).put(put_session_usage_limit),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::session::diverge_tests::{
        install_test_user_action_key, TEST_USER_ACTION_KEY,
    };
    use crate::routes::session_reach::SESSION_OUT_OF_REACH;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use biorouter::model::ModelConfig;
    use biorouter::privacy::SessionClassification;
    use serde_json::{json, Value};
    use serial_test::serial;
    use std::path::PathBuf;
    use tower::ServiceExt;

    const PROOF: (&str, &str) = ("X-User-Action", TEST_USER_ACTION_KEY);

    struct Seeded {
        state: Arc<AppState>,
        id: String,
    }

    impl Drop for Seeded {
        fn drop(&mut self) {
            let state = self.state.clone();
            let id = std::mem::take(&mut self.id);
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async move {
                    let _ = state.session_manager().delete_session(&id).await;
                })
            });
        }
    }

    async fn seed(state: &Arc<AppState>, tier: SessionClassification) -> Seeded {
        let manager = state.session_manager();
        let session = manager
            .create_session(
                PathBuf::from("/tmp/usage_limit_route"),
                "Usage limit route (test fixture)".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        if tier == SessionClassification::Private {
            manager
                .update(&session.id)
                .provider_name("versa_azure")
                .model_config(ModelConfig::new("gpt-4o").unwrap())
                .raise_privacy(SessionClassification::Private, "turn:versa_azure")
                .apply()
                .await
                .unwrap();
        }
        Seeded {
            state: state.clone(),
            id: session.id,
        }
    }

    async fn send(
        state: Arc<AppState>,
        method: &str,
        uri: &str,
        body: Option<Value>,
        headers: &[(&str, &str)],
    ) -> (StatusCode, String) {
        let mut builder = Request::builder().method(method).uri(uri);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let body = match body {
            Some(json) => {
                builder = builder.header("content-type", "application/json");
                Body::from(serde_json::to_vec(&json).unwrap())
            }
            None => Body::empty(),
        };
        let app = crate::routes::configure(state, "usage-limit-route-secret".to_string());
        let res = app.oneshot(builder.body(body).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn json_of(body: &str) -> Value {
        serde_json::from_str(body).unwrap_or_else(|e| panic!("not JSON ({e}): {body}"))
    }

    #[test]
    fn a_write_needs_the_proof_only_where_one_can_be_given() {
        assert_eq!(proof_refusal(UserActionProof::Proven, true), None);
        assert_eq!(
            proof_refusal(UserActionProof::Unproven, false),
            Some(USAGE_LIMIT_NEED_USER)
        );
        assert_eq!(
            proof_refusal(UserActionProof::NoKeyInstalled, true),
            Some(USAGE_LIMIT_KEY_MISSING)
        );
        assert_eq!(proof_refusal(UserActionProof::NoKeyInstalled, false), None);
    }

    #[test]
    fn a_request_body_maps_to_the_stored_override() {
        let request = |mode, max_tokens, max_usd| SetSessionUsageLimitRequest {
            mode,
            max_tokens,
            max_usd,
        };
        assert_eq!(
            requested_override(&request(UsageLimitMode::Inherit, None, None)),
            Ok(None)
        );
        assert_eq!(
            requested_override(&request(UsageLimitMode::Unlimited, None, None)),
            Ok(Some(ChatLimitOverride::Unlimited))
        );
        assert_eq!(
            requested_override(&request(UsageLimitMode::Limit, Some(10), None)),
            Ok(Some(ChatLimitOverride::Limit {
                max_tokens: Some(10),
                max_usd: None
            }))
        );
        for bad in [
            request(UsageLimitMode::Limit, None, None),
            request(UsageLimitMode::Limit, Some(0), None),
            request(UsageLimitMode::Limit, None, Some(-1.0)),
            request(UsageLimitMode::Limit, None, Some(f64::INFINITY)),
            request(UsageLimitMode::Unlimited, Some(5), None),
            request(UsageLimitMode::Inherit, None, Some(1.0)),
        ] {
            assert!(requested_override(&bad).is_err(), "{bad:?}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_public_chat_reads_and_a_person_sets_its_limit() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let chat = seed(&state, SessionClassification::Public).await;
        let uri = format!("/sessions/{}/usage-limit", chat.id);

        let (status, body) = send(state.clone(), "GET", &uri, None, &[]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let read = json_of(&body);
        assert!(read["chatOverride"].is_null(), "{body}");
        assert_eq!(read["used"]["chatTokens"], 0, "{body}");
        assert!(read.get("paused").is_some() && read.get("warn").is_some());
        assert_eq!(read["billingSessionId"], chat.id.as_str());

        // Without the proof a model cannot set, or lift, the limit.
        let (status, body) = send(
            state.clone(),
            "PUT",
            &uri,
            Some(json!({ "mode": "unlimited" })),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body, USAGE_LIMIT_NEED_USER);

        let (status, body) = send(
            state.clone(),
            "PUT",
            &uri,
            Some(json!({ "mode": "limit", "maxTokens": 1000 })),
            &[PROOF],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let stored = json_of(&body);
        assert_eq!(stored["limit"]["maxTokens"], 1000, "{body}");
        assert_eq!(stored["limit"]["source"], "chat", "{body}");
        assert_eq!(stored["chatOverride"]["mode"], "limit", "{body}");
        assert_eq!(stored["paused"], false, "{body}");

        let (status, body) = send(
            state.clone(),
            "PUT",
            &uri,
            Some(json!({ "mode": "inherit" })),
            &[PROOF],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(json_of(&body)["chatOverride"].is_null(), "{body}");

        for bad in [
            json!({ "mode": "limit" }),
            json!({ "mode": "limit", "maxTokens": -5 }),
            json!({ "mode": "sometimes" }),
            json!({ "mode": "unlimited", "extra": 1 }),
        ] {
            let (status, body) =
                send(state.clone(), "PUT", &uri, Some(bad.clone()), &[PROOF]).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn a_private_chat_is_refused_exactly_as_the_read_is() {
        install_test_user_action_key();
        let state = AppState::new().await.unwrap();
        let private = seed(&state, SessionClassification::Private).await;
        let uri = format!("/sessions/{}/usage-limit", private.id);

        let (status, body) = send(state.clone(), "GET", &uri, None, &[]).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body, SESSION_OUT_OF_REACH);
        let (status, body) = send(
            state.clone(),
            "PUT",
            &uri,
            Some(json!({ "mode": "unlimited" })),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body, SESSION_OUT_OF_REACH);

        let (status, body) = send(state.clone(), "GET", &uri, None, &[PROOF]).await;
        assert_eq!(status, StatusCode::OK, "{body}");

        let (status, _) = send(
            state.clone(),
            "GET",
            "/sessions/29990101_646464/usage-limit",
            None,
            &[PROOF],
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = send(state, "GET", "/sessions/not.a.chat/usage-limit", None, &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
