//! `GET /crew/changes`, and the layer that announces the Crew routes' own changes on it (chat
//! parity, WI-15).
//!
//! A Crew view reads this computer's Crew state when it opens and after its own actions. The
//! feed (`biorouter::crew::changes`) is how it learns that something changed anywhere else: a
//! chat's `crew__change`, a setup job moving, another window, `biorouter crew` in a terminal. The
//! last two reach the daemon through the routes below, so [`announcing`] wraps them: a request
//! that changed something and succeeded is announced with the IDs its path names.
//!
//! The route answers IDs only (see the feed's module docs), and only to a person: the IDs it
//! names (saved connections, chats) are otherwise read through routes that need the person's
//! proof, so a caller holding just the daemon's secret learns nothing here it could not before.

use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{MatchedPath, Query, Request};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use biorouter::crew::changes::{self, ChangeIds, CrewChangesDelta};
use biorouter_server::auth::{user_action_proof, UserActionProof};
use serde::Deserialize;
use utoipa::IntoParams;

use super::crew::wire::CrewError;
use super::crew_authentication::{
    no_human_authority, HUMAN_AUTHORITY_UNAVAILABLE_CODE, USER_ACTION_REQUIRED_CODE,
};

/// The longest a poll waits before answering with nothing new. Under the 30 seconds most
/// clients and proxies give up at.
const MAX_WAIT: Duration = Duration::from_secs(25);

/// The route that sends one protocol request to a workspace: a change only when it carries an
/// idempotency key (`request_id`), which every client sends with a change and never with a read.
const WORKSPACE_REQUEST: &str = "/crew/connections/{id}/request";
/// The most of a workspace request's body read to find its `request_id`: the route's own limit.
const WORKSPACE_REQUEST_LIMIT: usize = 2 * 1024 * 1024;
/// The workspace changes nothing on this feed shows: a read marker, which an open channel sends
/// by itself as messages arrive. Announced, each would have every open Crew view read its lists
/// again for every message. Unread counts reach the other windows through their own observation.
const UNSHOWN_WORKSPACE_CHANGES: [&str; 1] = ["channel.read"];

/// The Crew routes that take a POST but change nothing: watching a workspace, resolving names,
/// planning a sign-in and previewing an image. They are asked often, and a change announced for
/// each would have every open Crew view read everything again for nothing.
const READS_BY_POST: [&str; 4] = [
    "/crew/connections/{id}/observe",
    "/crew/resolve",
    "/crew/connections/{id}/auth-plan",
    "/crew/transfers/preview",
];

/// The kind a change made through a route is announced with, when the route does what one
/// `crew__change` action does: a client reacts to `rejoin` the same way wherever it came from.
/// Any other change a route makes is announced as `route`.
const ROUTE_KINDS: [(&str, &str, &str); 21] = [
    ("POST", "/crew/connections/{id}/rejoin", "rejoin"),
    ("POST", "/crew/connections/{id}/join", "join"),
    ("POST", "/crew/connections/{id}/connect", "connect"),
    ("POST", "/crew/connections/{id}/disconnect", "disconnect"),
    ("PATCH", "/crew/connections/{id}", "connections_update"),
    ("DELETE", "/crew/connections/{id}", "connections_remove"),
    (
        "POST",
        "/crew/connections/{id}/workspace/delete",
        "workspace_delete",
    ),
    (
        "POST",
        "/crew/connections/{id}/sessions/{session_id}/revoke",
        "grants_revoke",
    ),
    (
        "DELETE",
        "/crew/sessions/{session_id}/reads/{connection_id}",
        "grants_revoke",
    ),
    (
        "POST",
        "/crew/connections/{id}/devices/acknowledge",
        "profile_devices_acknowledge",
    ),
    (
        "POST",
        "/crew/connections/{id}/context/draft",
        "context_draft",
    ),
    ("POST", "/crew/connections/{id}/runs", "tasks_ask"),
    (
        "POST",
        "/crew/connections/{id}/runs/{run_id}/cancel",
        "tasks_cancel",
    ),
    ("POST", "/crew/setup", "setup"),
    ("DELETE", "/crew/setup/{job_id}", "setup_cancel"),
    ("POST", "/crew/transfers/{id}/pause", "files_pause"),
    ("POST", "/crew/transfers/{id}/resume", "files_resume"),
    ("DELETE", "/crew/transfers/{id}", "files_forget"),
    ("POST", "/crew/credentials/unlock", "credentials_unlock"),
    ("POST", "/crew/credentials/init", "credentials_init"),
    ("POST", "/crew/credentials/lock", "credentials_lock"),
];
/// The kind of a route's change that no `crew__change` action matches.
const ROUTE_KIND: &str = "route";

#[derive(Debug, Deserialize, IntoParams)]
pub struct CrewChangesQuery {
    /// The last revision this reader saw. Absent: answer where the feed stands now, at once and
    /// with no changes, which is how a new reader learns where to start.
    #[serde(default)]
    since: Option<u64>,
    /// How long to wait for a change, in milliseconds, at most 25 000 (the default). `0`
    /// answers at once.
    #[serde(default)]
    timeout_ms: Option<u64>,
}

/// The person gate's refusal, with the Crew routes' own codes and words.
fn refusal(code: &str, error: &str) -> Response {
    let body = CrewError {
        code: code.to_owned(),
        error: error.to_owned(),
        ..CrewError::default()
    };
    (StatusCode::FORBIDDEN, Json(body)).into_response()
}

/// What changed in Crew on this computer after `since`: IDs and kinds only.
#[utoipa::path(
    get,
    operation_id = "crew_changes",
    path = "/crew/changes",
    params(CrewChangesQuery),
    responses(
        (status = 200, description = "The changes after `since`, by kind and ID only: a `crew__change` that ran, a change a Crew route made, or a setup job's new state. With no `since`, where the feed stands now and no changes. `truncated` means the reader fell behind or the daemon restarted: read everything again. A change names no name and no content; read what changed through the route that shows it", body = CrewChangesDelta),
        (status = 403, description = "No proof that a person asked: `crew_user_action_required`, or `crew_human_authority_unavailable` on a daemon that holds no user-action key", body = CrewError)
    ),
    tag = "Crew"
)]
pub async fn crew_changes(
    headers: HeaderMap,
    Query(query): Query<CrewChangesQuery>,
) -> Result<Json<CrewChangesDelta>, Response> {
    match user_action_proof(&headers) {
        UserActionProof::Proven => {}
        UserActionProof::Unproven => {
            return Err(refusal(
                USER_ACTION_REQUIRED_CODE,
                "Only a person using the Biorouter desktop app or the biorouter crew command can do this. Agent tools use their separate task grant.",
            ))
        }
        UserActionProof::NoKeyInstalled => {
            return Err(refusal(
                HUMAN_AUTHORITY_UNAVAILABLE_CODE,
                no_human_authority("This daemon cannot verify human Crew actions. Start it from the Biorouter desktop app or with biorouter crew daemon start."),
            ))
        }
    }
    let feed = changes::feed();
    let Some(since) = query.since else {
        return Ok(Json(feed.current()));
    };
    let wait = query
        .timeout_ms
        .map_or(MAX_WAIT, Duration::from_millis)
        .min(MAX_WAIT);
    Ok(Json(feed.wait(since, wait).await))
}

pub fn routes() -> Router {
    Router::new().route("/crew/changes", get(crew_changes))
}

/// `router` with every change its routes make announced on Crew's change feed
/// ([`announce_changes`]). Layered per route, so only a request that reached one of them counts.
pub fn announcing(router: Router) -> Router {
    router.route_layer(axum::middleware::from_fn(announce_changes))
}

/// Announce a request that changed something and succeeded: a POST, PUT, PATCH or DELETE to one
/// of the Crew routes, answered 2xx, except the reads that take a POST ([`READS_BY_POST`]), a
/// workspace request without an idempotency key ([`WORKSPACE_REQUEST`]) and a read marker
/// ([`UNSHOWN_WORKSPACE_CHANGES`]). The answer itself is passed on untouched.
pub async fn announce_changes(request: Request, next: Next) -> Response {
    let Some(template) = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned())
    else {
        return next.run(request).await;
    };
    let method = request.method().clone();
    if !changes_by_method(&method) || READS_BY_POST.contains(&template.as_str()) {
        return next.run(request).await;
    }
    let ids = path_ids(&template, request.uri().path());
    let (request, changes_something) = if template == WORKSPACE_REQUEST {
        carries_request_id(request).await
    } else {
        (request, true)
    };
    let response = next.run(request).await;
    if changes_something && response.status().is_success() {
        changes::publish(route_kind(&method, &template), ids);
    }
    response
}

fn changes_by_method(method: &Method) -> bool {
    [Method::POST, Method::PUT, Method::PATCH, Method::DELETE].contains(method)
}

fn route_kind(method: &Method, template: &str) -> &'static str {
    ROUTE_KINDS
        .iter()
        .find(|(verb, path, _)| method.as_str() == *verb && template == *path)
        .map_or(ROUTE_KIND, |(_, _, kind)| kind)
}

/// The IDs a request's path names, by its route's parameters. `{id}` is a saved connection's
/// only under `/crew/connections/`; elsewhere it is a transfer's or a sign-in's, and not
/// announced. Each is checked to look like an ID when it is published.
fn path_ids(template: &str, path: &str) -> ChangeIds {
    let mut ids = ChangeIds::default();
    let connection_route = template.starts_with("/crew/connections/{id}");
    for (part, value) in template.split('/').zip(path.split('/')) {
        let slot = match part {
            "{id}" if connection_route => &mut ids.connection_id,
            "{connection_id}" => &mut ids.connection_id,
            "{session_id}" => &mut ids.session_id,
            "{run_id}" => &mut ids.run_id,
            "{job_id}" => &mut ids.job_id,
            _ => continue,
        };
        *slot = Some(value.to_owned());
    }
    ids
}

/// Whether a workspace request is a change this feed announces: it carries an idempotency key
/// (`request_id`) and is not a read marker ([`UNSHOWN_WORKSPACE_CHANGES`]). Answers the request to
/// pass on with it. The body is read only when it says how long it is and is within the route's
/// own limit; otherwise the request is passed on as it came and counted as a change, which the
/// route's answer then decides.
async fn carries_request_id(request: Request) -> (Request, bool) {
    let declared = request
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    if declared.is_none_or(|length| length > WORKSPACE_REQUEST_LIMIT) {
        return (request, true);
    }
    let (parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, WORKSPACE_REQUEST_LIMIT).await {
        Ok(bytes) => bytes,
        // A body that could not be read is refused by the route too: nothing changed.
        Err(_) => Bytes::new(),
    };
    let shown = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .is_some_and(|body| {
            let keyed = body
                .get("request_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|key| !key.is_empty());
            let method = body.get("method").and_then(serde_json::Value::as_str);
            keyed && !method.is_some_and(|method| UNSHOWN_WORKSPACE_CHANGES.contains(&method))
        });
    (Request::from_parts(parts, Body::from(bytes)), shown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::session::diverge_tests::{
        install_test_user_action_key, TEST_USER_ACTION_KEY,
    };
    use axum::http::StatusCode;
    use axum::routing::{delete, patch, post};
    use biorouter::crew::changes::{feed, CrewChange};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    async fn ok() -> StatusCode {
        StatusCode::OK
    }

    async fn refused() -> StatusCode {
        StatusCode::CONFLICT
    }

    async fn echo(body: Bytes) -> Bytes {
        body
    }

    /// The real Crew paths, each answered by a stand-in, behind the real layer.
    fn crew_like() -> Router {
        announcing(
            Router::new()
                .route("/crew/connections", get(ok).post(ok))
                .route("/crew/connections/{id}", patch(ok).delete(ok))
                .route("/crew/connections/{id}/rejoin", post(ok))
                .route("/crew/connections/{id}/connect", post(refused))
                .route("/crew/connections/{id}/observe", post(ok))
                .route(WORKSPACE_REQUEST, post(echo))
                .route(
                    "/crew/sessions/{session_id}/reads/{connection_id}",
                    delete(ok),
                )
                .route("/crew/transfers/{id}", delete(ok)),
        )
        .merge(routes())
    }

    fn send(method: &str, uri: &str, body: Option<Value>) -> Request {
        let builder = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header("X-User-Action", TEST_USER_ACTION_KEY);
        match body {
            Some(body) => {
                let text = body.to_string();
                builder
                    .header("content-type", "application/json")
                    .header("content-length", text.len())
                    .body(Body::from(text))
                    .unwrap()
            }
            None => builder.body(Body::empty()).unwrap(),
        }
    }

    /// What the feed holds after `from` for the connection `connection`.
    fn announced(from: u64, connection: &str) -> Vec<CrewChange> {
        feed()
            .since(from)
            .changes
            .into_iter()
            .filter(|change| change.connection_id.as_deref() == Some(connection))
            .collect()
    }

    fn unique() -> String {
        format!("c-{}", uuid::Uuid::new_v4())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_change_a_route_made_is_announced_and_a_read_is_not() {
        let app = crew_like();
        let connection = unique();
        let from = feed().revision();
        for (method, path, status) in [
            ("GET", "/crew/connections".to_owned(), StatusCode::OK),
            (
                "POST",
                format!("/crew/connections/{connection}/observe"),
                StatusCode::OK,
            ),
            // Refused: nothing changed.
            (
                "POST",
                format!("/crew/connections/{connection}/connect"),
                StatusCode::CONFLICT,
            ),
        ] {
            let response = app
                .clone()
                .oneshot(send(method, &path, None))
                .await
                .unwrap();
            assert_eq!(response.status(), status, "{method} {path}");
        }
        assert!(announced(from, &connection).is_empty());

        let response = app
            .clone()
            .oneshot(send(
                "POST",
                &format!("/crew/connections/{connection}/rejoin"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response = app
            .clone()
            .oneshot(send(
                "DELETE",
                &format!("/crew/connections/{connection}"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let kinds: Vec<String> = announced(from, &connection)
            .into_iter()
            .map(|change| change.kind)
            .collect();
        assert_eq!(kinds, ["rejoin", "connections_remove"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_route_s_ids_come_from_its_path() {
        let app = crew_like();
        let connection = unique();
        let from = feed().revision();
        let response = app
            .clone()
            .oneshot(send(
                "DELETE",
                &format!("/crew/sessions/20261106_7/reads/{connection}"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let changes = announced(from, &connection);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, "grants_revoke");
        assert_eq!(changes[0].session_id.as_deref(), Some("20261106_7"));
        // A transfer's `{id}` is not a connection's.
        let transfer = unique();
        let from = feed().revision();
        app.clone()
            .oneshot(send("DELETE", &format!("/crew/transfers/{transfer}"), None))
            .await
            .unwrap();
        let since = feed().since(from);
        assert!(since
            .changes
            .iter()
            .any(|change| change.kind == "files_forget"));
        assert!(announced(from, &transfer).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_workspace_request_is_a_change_only_with_its_idempotency_key() {
        let app = crew_like();
        let connection = unique();
        let path = format!("/crew/connections/{connection}/request");
        let from = feed().revision();
        let read = json!({"method": "workspace.snapshot", "params": {}, "request_id": null});
        let response = app
            .clone()
            .oneshot(send("POST", &path, Some(read.clone())))
            .await
            .unwrap();
        // The route reads the body the layer read: it is passed on whole.
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), read);
        assert!(announced(from, &connection).is_empty());

        // A read marker changes nothing the feed shows, though it carries a key.
        let marker = json!({"method": "channel.read", "params": {"channel_id": "c", "sequence": "9"}, "request_id": "k-0"});
        app.clone()
            .oneshot(send("POST", &path, Some(marker)))
            .await
            .unwrap();
        assert!(announced(from, &connection).is_empty());

        let write =
            json!({"method": "channel.create", "params": {"name": "x"}, "request_id": "k-1"});
        app.clone()
            .oneshot(send("POST", &path, Some(write)))
            .await
            .unwrap();
        let changes = announced(from, &connection);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, ROUTE_KIND);
    }

    async fn poll(app: &Router, query: &str, proof: bool) -> (StatusCode, Value) {
        let mut request = axum::http::Request::builder().uri(format!("/crew/changes{query}"));
        if proof {
            request = request.header("X-User-Action", TEST_USER_ACTION_KEY);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_poll_answers_early_on_a_change_and_at_its_timeout_otherwise() {
        install_test_user_action_key();
        let app = routes();
        let (status, baseline) = poll(&app, "", true).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(baseline["changes"], json!([]));
        let since = baseline["revision"].as_u64().unwrap();

        // Behind already: answered at once, whatever the timeout.
        let connection = unique();
        changes::publish(
            "channels_create",
            ChangeIds {
                connection_id: Some(connection.clone()),
                ..ChangeIds::default()
            },
        );
        let started = std::time::Instant::now();
        let (_, delta) = poll(&app, &format!("?since={since}&timeout_ms=20000"), true).await;
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(delta["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|change| change["connection_id"] == json!(connection)));

        // A parked poll ends when a change lands.
        let current = delta["revision"].as_u64().unwrap();
        let parked = {
            let app = app.clone();
            tokio::spawn(async move {
                poll(&app, &format!("?since={current}&timeout_ms=20000"), true).await
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        changes::publish("rejoin", ChangeIds::default());
        let (_, woke) = tokio::time::timeout(Duration::from_secs(5), parked)
            .await
            .expect("the poll ended when the change landed")
            .unwrap();
        assert!(woke["revision"].as_u64().unwrap() > current);

        // Nothing lands: answered at its timeout, with nothing.
        let quiet = feed().revision();
        let started = std::time::Instant::now();
        let (status, idle) = poll(&app, &format!("?since={quiet}&timeout_ms=40"), true).await;
        let waited = started.elapsed();
        assert_eq!(status, StatusCode::OK);
        assert!(waited < MAX_WAIT);
        // The feed is the process's own, and another test may announce meanwhile: then the poll
        // ends early, with only what came after. With nothing new, it waited out its timeout.
        let listed = idle["changes"].as_array().unwrap();
        assert!(listed
            .iter()
            .all(|change| change["revision"].as_u64().unwrap() > quiet));
        assert!(
            !listed.is_empty() || waited >= Duration::from_millis(30),
            "answered after {waited:?} with nothing new"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_feed_answers_only_a_person_and_names_ids_only() {
        install_test_user_action_key();
        let app = routes();
        let (status, refusal) = poll(&app, "", false).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(refusal["code"], json!(USER_ACTION_REQUIRED_CODE));

        // A private chat's change: its IDs, and no name it was given.
        let from = feed().revision();
        changes::publish(
            "grants_revoke",
            ChangeIds {
                connection_id: Some("Lab notes".into()),
                session_id: Some("20261106_9".into()),
                ..ChangeIds::default()
            },
        );
        let (status, delta) = poll(&app, &format!("?since={from}&timeout_ms=0"), true).await;
        assert_eq!(status, StatusCode::OK);
        let change = delta["changes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|change| change["session_id"] == json!("20261106_9"))
            .expect("the change is listed")
            .clone();
        assert_eq!(change["connection_id"], Value::Null);
        let mut keys: Vec<&str> = change
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "channel_id",
                "connection_id",
                "job_id",
                "kind",
                "revision",
                "run_id",
                "session_id"
            ]
        );
    }

    #[test]
    fn every_route_kind_is_a_change_action_on_a_real_route() {
        let spec: Value = serde_json::from_str(&crate::openapi::generate_schema()).unwrap();
        for (method, path, kind) in ROUTE_KINDS {
            assert!(
                biorouter::crew::tool_host::CHANGE_ACTIONS.contains(&kind),
                "{kind} is not a crew__change action"
            );
            assert!(
                !spec["paths"][path][method.to_lowercase()].is_null(),
                "{method} {path} is not a Crew route"
            );
        }
        for path in READS_BY_POST.into_iter().chain([WORKSPACE_REQUEST]) {
            assert!(
                !spec["paths"][path]["post"].is_null(),
                "POST {path} is not a Crew route"
            );
        }
        assert!(!spec["paths"]["/crew/changes"]["get"].is_null());
    }
}
