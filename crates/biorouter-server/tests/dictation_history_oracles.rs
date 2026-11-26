//! The dictation history's existence oracle rules (DD-96; 7.5 of the history design) on a
//! daemon that holds a user action key:
//!
//! * an id the caller may not list answers byte for byte as one that names nothing, and
//!   as one that could not name an entry at all (rule 1);
//! * a private entry appended changes neither the `ETag` nor the `total` a public caller
//!   polls, so a poll never says a private dictation was made (rules 2 and 3);
//! * no refusal names a dictation's text or id (rule 5), and the tier route refuses from
//!   the request before any entry is read (rule 7).
//!
//! The tests share the process's one history, so they run one at a time.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::Router;
use biorouter::dictation::history::{self as history, EntryTier, HistoryEntry, Surface};
use biorouter_server::routes::dictation_history;
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tower::ServiceExt;

const SECRET: &str = "dictation-history-oracles-secret";
const USER_KEY: &str = "dictation-history-oracles-user-action-key";
const SENTINEL: &str = "ORACLE-SENTINEL-41c9";

#[derive(Clone, Copy, Debug)]
enum Caller {
    SecretOnly,
    StatedPrivate,
    Proof,
}

const PUBLIC_CALLERS: [Caller; 2] = [Caller::SecretOnly, Caller::StatedPrivate];

async fn daemon() -> Router {
    let digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(USER_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));
    let _ = std::fs::remove_dir_all(history::history_dir());
    let state = AppState::new().await.expect("app state");
    dictation_history::routes(Arc::clone(&state)).layer(axum::middleware::from_fn_with_state(
        SECRET.to_string(),
        biorouter_server::auth::check_token,
    ))
}

struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    raw: Bytes,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.raw).unwrap_or(Value::Null)
    }
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    caller: Caller,
    extra: &[(&str, &str)],
    body: Option<Value>,
) -> Answer {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Secret-Key", SECRET)
        .header(header::CONTENT_TYPE, "application/json");
    builder = match caller {
        Caller::SecretOnly => builder,
        Caller::StatedPrivate => builder.header("X-Caller-Provider", "versa_azure"),
        Caller::Proof => builder.header("X-User-Action", USER_KEY),
    };
    for (name, value) in extra {
        builder = builder.header(*name, *value);
    }
    let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let raw = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    Answer {
        status,
        headers,
        raw,
    }
}

fn add(text: &str, tier: EntryTier) -> String {
    let entry = HistoryEntry {
        id: history::store::new_entry_id(),
        created_at: chrono::Utc::now(),
        text: text.to_string(),
        tier,
        institutions: None,
        surface: Surface::Composer,
        duration_ms: 2_000,
        model: "for-tests".to_string(),
        private_terms: tier.is_private(),
        session_id: None,
        session_incarnation: None,
        truncated: None,
    };
    let id = entry.id.clone();
    history::open_history().append(entry).expect("appended");
    id
}

/// Rule 1: a private entry's id, a made up one and one that could not name an entry all
/// answer one 404, byte for byte, on every route that names an entry.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn an_unlistable_id_answers_exactly_as_a_missing_one() {
    let app = daemon().await;
    let private = add(&format!("{SENTINEL} private"), EntryTier::Private);
    let missing = history::store::new_entry_id();
    for caller in PUBLIC_CALLERS {
        let mut answers = Vec::new();
        for id in [private.as_str(), missing.as_str(), "dch_not-a-uuid", "x"] {
            let answer = send(
                &app,
                "DELETE",
                &format!("/dictation/history/{id}"),
                caller,
                &[],
                None,
            )
            .await;
            assert_eq!(answer.status, StatusCode::NOT_FOUND, "{caller:?} {id}");
            answers.push(answer.raw);
        }
        assert!(
            answers.windows(2).all(|pair| pair[0] == pair[1]),
            "{caller:?}: the answers differ: {answers:?}"
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&answers[0]).unwrap(),
            json!({"code": "not_found", "message": dictation_history::MSG_HISTORY_NOT_FOUND})
        );
    }
    assert!(history::open_history().get(&private).unwrap().is_some());
    // The person reaches it.
    let deleted = send(
        &app,
        "DELETE",
        &format!("/dictation/history/{private}"),
        Caller::Proof,
        &[],
        None,
    )
    .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
}

/// Rules 2 and 3: a public caller's page, its `ETag` and its `total` are the same before
/// and after a private dictation lands, and a poll with the tag answers 304; the person's
/// own tag moves.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_private_append_changes_nothing_a_public_caller_polls() {
    let app = daemon().await;
    add("a public dictation", EntryTier::Public);
    let mut before = Vec::new();
    for caller in PUBLIC_CALLERS {
        let answer = send(&app, "GET", "/dictation/history", caller, &[], None).await;
        assert_eq!(answer.status, StatusCode::OK);
        assert_eq!(answer.json()["total"], 1);
        before.push(answer);
    }
    let person_before = send(&app, "GET", "/dictation/history", Caller::Proof, &[], None).await;

    add(&format!("{SENTINEL} private"), EntryTier::Private);

    for (caller, earlier) in PUBLIC_CALLERS.into_iter().zip(&before) {
        let tag = earlier.headers[header::ETAG].to_str().unwrap().to_string();
        let now = send(&app, "GET", "/dictation/history", caller, &[], None).await;
        assert_eq!(now.status, StatusCode::OK, "{caller:?}");
        assert_eq!(now.raw, earlier.raw, "{caller:?}: the page moved");
        assert_eq!(now.headers[header::ETAG], tag.as_str(), "{caller:?}");
        assert_eq!(now.json()["total"], 1, "{caller:?}");
        let polled = send(
            &app,
            "GET",
            "/dictation/history",
            caller,
            &[("if-none-match", tag.as_str())],
            None,
        )
        .await;
        assert_eq!(polled.status, StatusCode::NOT_MODIFIED, "{caller:?}");
        assert!(polled.raw.is_empty());
        // A search for the private text finds nothing, and counts nothing.
        let searched = send(
            &app,
            "GET",
            &format!("/dictation/history?q={SENTINEL}"),
            caller,
            &[],
            None,
        )
        .await;
        assert_eq!(searched.json()["total"], 0, "{caller:?}");
        assert!(!String::from_utf8_lossy(&searched.raw).contains(SENTINEL));
    }
    let person_after = send(&app, "GET", "/dictation/history", Caller::Proof, &[], None).await;
    assert_ne!(
        person_after.headers[header::ETAG],
        person_before.headers[header::ETAG],
        "the person's own page moved"
    );
    assert_eq!(person_after.json()["total"], 2);
}

/// Rules 5 and 7: every refusal of every route is a code and a constant sentence, and
/// none names an entry's text or id; the tier route answers a caller without the proof
/// the same for every id, before it reads one.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn refusals_name_no_text_and_no_id() {
    let app = daemon().await;
    let public = add(&format!("{SENTINEL} public"), EntryTier::Public);
    let private = add(&format!("{SENTINEL} private"), EntryTier::Private);
    let missing = history::store::new_entry_id();
    let mut refusals = Vec::new();
    for caller in PUBLIC_CALLERS {
        let mut tier_answers = Vec::new();
        for id in [&public, &private, &missing] {
            let answer = send(
                &app,
                "POST",
                &format!("/dictation/history/{id}/tier"),
                caller,
                &[],
                Some(json!({"tier": "public"})),
            )
            .await;
            assert_eq!(answer.status, StatusCode::FORBIDDEN, "{caller:?}");
            tier_answers.push(answer.raw.clone());
            refusals.push(answer);
        }
        assert!(
            tier_answers.windows(2).all(|pair| pair[0] == pair[1]),
            "{caller:?}: the tier route read the id before refusing"
        );
        for id in [&public, &private, &missing] {
            refusals.push(
                send(
                    &app,
                    "DELETE",
                    &format!("/dictation/history/{id}"),
                    caller,
                    &[],
                    None,
                )
                .await,
            );
        }
        for (uri, body) in [
            (
                "/dictation/history/delete",
                json!({"ids": [&public, &private]}),
            ),
            (
                "/dictation/history/clear",
                json!({"through": chrono::Utc::now()}),
            ),
            (
                "/dictation/history/settings",
                json!({"keep_history": false}),
            ),
        ] {
            refusals.push(send(&app, "POST", uri, caller, &[], Some(body)).await);
        }
    }
    // Malformed requests, from the person.
    for (method, uri, body) in [
        ("GET", "/dictation/history?cursor=nope".to_string(), None),
        ("GET", "/dictation/history?limit=-1".to_string(), None),
        (
            "POST",
            format!("/dictation/history/{private}/tier"),
            Some(json!({"tier": "secret"})),
        ),
        (
            "POST",
            "/dictation/history/clear".to_string(),
            Some(json!({"through": "yesterday"})),
        ),
    ] {
        refusals.push(send(&app, method, &uri, Caller::Proof, &[], body).await);
    }
    for refusal in &refusals {
        assert!(
            refusal.status.is_client_error(),
            "{}: {:?}",
            refusal.status,
            refusal.raw
        );
        let text = String::from_utf8_lossy(&refusal.raw);
        for secret in [
            SENTINEL,
            public.as_str(),
            private.as_str(),
            missing.as_str(),
        ] {
            assert!(!text.contains(secret), "a refusal named {secret}: {text}");
        }
        let body = refusal.json();
        let fields: Vec<&str> = body
            .as_object()
            .map(|fields| fields.keys().map(String::as_str).collect())
            .unwrap_or_default();
        assert_eq!(fields, ["code", "message"], "{text}");
    }
    // Nothing was written by any of them.
    let held = history::open_history();
    assert_eq!(held.get(&public).unwrap().unwrap().tier, EntryTier::Public);
    assert_eq!(
        held.get(&private).unwrap().unwrap().tier,
        EntryTier::Private
    );
    assert!(held.settings().keep_history);
}
