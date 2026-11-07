//! `/dictionary/*` on a daemon that holds a user action key, as the desktop's own daemon
//! does (PR-16; `docs/dictation/dictionary.md`, "Routes", "Refusals", "Who stamps the
//! tier", "Private rows and institutions", "The observations route"):
//!
//! * listing by standing, an `ETag` over the caller's own page, `If-None-Match`;
//! * every write needs the proof, and a caller without it is refused with
//!   MSG-DICTIONARY-AGENT;
//! * add, edit (with `If-Match`), delete, bulk delete over listed ids, keep, accept,
//!   dismiss, import (dry run and merge), export, the tier change, the settings;
//! * a private row's institutions: an add with no origin gives `null`, one with
//!   `origin_unresolved` gives `[]`, one whose origin chat has no live agent or an
//!   unreadable record gives `[]`, and one from a chat on a local model whose record holds
//!   `ucsf` gives `{ucsf}` (O-35, O-36);
//! * observations: a correction in a chat bound to a private model whose row is still
//!   public lands in a private row, as does one in a live chat whose model is not read;
//!   resends count once; secret and PII pairs are dropped; a row whose institutions do not
//!   admit the chat's model is left as it was;
//! * managed `dictionary: false` refuses writes and observations with MSG-POLICY while
//!   delete, the bulk delete, dismiss and export stay allowed, and managed
//!   `dictionary_agent_writes: false` refuses raising the settings; there is no
//!   `POST /dictionary/clear`.
//!
//! Its own binary, because the installed digest is a process global. No test here reaches
//! the network: the providers bound to chats are built, never asked anything.
//! The policy tests are Unix only, for the file modes the policy's trust check reads.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::Router;
use biorouter::config::with_config_overrides;
use biorouter::model::ModelConfig;
use biorouter::privacy::affiliation::InstitutionId;
use biorouter::privacy::SessionClassification;
use biorouter::providers::versa_azure::VERSA_AZURE_DEFAULT_MODEL;
use biorouter::security::dictionary_store::{self, store};
use biorouter::session::session_manager::{SessionManager, DB_NAME, SESSIONS_FOLDER};
use biorouter::session::SessionType;
use biorouter_server::routes::dictionary::{self, MSG_DICTIONARY_AGENT};
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use store::{EntrySource, EntryStatus, EntryTier, NewTerm, Reach, RowAffiliation};
use tower::ServiceExt;

const SECRET: &str = "dictionary-keyed-secret";
const USER_KEY: &str = "dictionary-keyed-user-action-key";

async fn daemon() -> (Arc<AppState>, Router) {
    let digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(USER_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));
    let _ = std::fs::remove_dir_all(dictionary_store::dictionary_dir());
    let state = AppState::new().await.expect("app state");
    let app = dictionary::routes(Arc::clone(&state)).layer(axum::middleware::from_fn_with_state(
        SECRET.to_string(),
        biorouter_server::auth::check_token,
    ));
    (state, app)
}

#[derive(Clone, Copy, Debug)]
enum Caller {
    SecretOnly,
    /// A private provider stated without the proof: standing, never authority.
    StatedPrivate,
    Proof,
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    caller: Caller,
    body: Option<Value>,
    extra: &[(&str, String)],
) -> Reply {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Secret-Key", SECRET)
        .header("content-type", "application/json");
    builder = match caller {
        Caller::SecretOnly => builder,
        Caller::StatedPrivate => builder.header("X-Caller-Provider", "versa_azure"),
        Caller::Proof => builder.header("X-User-Action", USER_KEY),
    };
    for (name, value) in extra {
        builder = builder.header(*name, value.as_str());
    }
    let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    Reply {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

async fn get(app: &Router, uri: &str, caller: Caller) -> Reply {
    send(app, "GET", uri, caller, None, &[]).await
}

async fn post(app: &Router, uri: &str, caller: Caller, body: Value) -> Reply {
    send(app, "POST", uri, caller, Some(body), &[]).await
}

/// Add `written` as the person, in `tier`; its id.
async fn add(app: &Router, written: &str, private: bool) -> String {
    let reply = post(
        app,
        "/dictionary/entries",
        Caller::Proof,
        json!({"written": written, "private": private}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    reply.body["entry"]["id"].as_str().unwrap().to_string()
}

fn written(list: &Value) -> Vec<String> {
    let mut out: Vec<String> = list["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["written"].as_str().unwrap().to_string())
        .collect();
    out.sort();
    out
}

fn store_rows() -> Vec<store::Entry> {
    dictionary_store::open_store()
        .view()
        .unwrap()
        .entries()
        .to_vec()
}

fn row(written: &str, tier: EntryTier) -> Option<store::Entry> {
    store_rows()
        .into_iter()
        .find(|e| e.written == written && e.tier == tier)
}

/// DD-37: public rows to every caller, private rows only to the proof or a stated
/// private provider; usage figures and `suggested_in` only with private standing; an
/// `ETag` over the caller's page, which a private write does not move for a public caller.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn listing_answers_by_standing_with_an_etag_over_the_callers_page() {
    let (_state, app) = daemon().await;
    add(&app, "EPAS1", false).await;
    add(&app, "PHD2", true).await;

    let proven = get(&app, "/dictionary/entries", Caller::Proof).await;
    assert_eq!(proven.status, StatusCode::OK);
    assert_eq!(written(&proven.body), ["EPAS1", "PHD2"]);
    assert_eq!(proven.body["entries"][0]["uses"], 0, "usage with standing");
    let phd2 = proven.body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["written"] == "PHD2")
        .cloned()
        .unwrap();
    assert_eq!(
        phd2["came_from_a_chat"], false,
        "a person's add with no chat shows none"
    );
    assert!(
        proven.body["entries"][0].get("came_from_a_chat").is_none()
            || proven.body["entries"][0]["tier"] == "private",
        "only private rows say whether they came from a chat"
    );
    let stated = get(&app, "/dictionary/entries", Caller::StatedPrivate).await;
    assert_eq!(written(&stated.body), ["EPAS1", "PHD2"]);
    let public = get(&app, "/dictionary/entries", Caller::SecretOnly).await;
    assert_eq!(written(&public.body), ["EPAS1"]);
    let entry = public.body["entries"][0].as_object().unwrap();
    for field in [
        "uses",
        "last_used_at",
        "corrected_to",
        "suggested_in",
        "came_from_a_chat",
        "seen",
    ] {
        assert!(!entry.contains_key(field), "{field} for a public caller");
    }
    assert!(public.body.get("total").is_none() && public.body.as_object().unwrap().len() == 2);

    let tag = public.headers[header::ETAG].to_str().unwrap().to_string();
    let again = send(
        &app,
        "GET",
        "/dictionary/entries",
        Caller::SecretOnly,
        None,
        &[("If-None-Match", tag.clone())],
    )
    .await;
    assert_eq!(again.status, StatusCode::NOT_MODIFIED);
    add(&app, "EGLN1", true).await;
    let after = get(&app, "/dictionary/entries", Caller::SecretOnly).await;
    assert_eq!(
        after.headers[header::ETAG].to_str().unwrap(),
        tag,
        "a private write never moves what a public caller polls"
    );
    let proven_after = get(&app, "/dictionary/entries", Caller::Proof).await;
    assert_ne!(
        proven_after.headers[header::ETAG],
        proven.headers[header::ETAG]
    );

    let suggested = get(&app, "/dictionary/entries?status=suggested", Caller::Proof).await;
    assert!(written(&suggested.body).is_empty());
    let found = get(&app, "/dictionary/entries?q=egl", Caller::Proof).await;
    assert_eq!(written(&found.body), ["EGLN1"]);
    let bad = get(&app, "/dictionary/entries?cursor=nonsense", Caller::Proof).await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    // A query string these routes cannot read is the module's own refusal, `{code,
    // message}`, never axum's plain text 400.
    for uri in [
        "/dictionary/entries?status=bogus",
        "/dictionary/export?include_private=maybe",
    ] {
        let bad = get(&app, uri, Caller::Proof).await;
        assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY, "{uri}");
        assert_eq!(bad.body["code"], "bad_request", "{uri}: {}", bad.body);
    }
}

/// A listing is paged at 500 rows, and its cursor walks the rest with nothing twice.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_listing_pages_by_cursor() {
    let (_state, app) = daemon().await;
    let lines: String = (0..620).map(|i| format!("TERM{i}\n")).collect();
    let imported = post(
        &app,
        "/dictionary/import",
        Caller::Proof,
        json!({"format": "lines", "content": lines}),
    )
    .await;
    assert_eq!(imported.body["new"], 620, "{}", imported.body);
    let first = get(&app, "/dictionary/entries", Caller::Proof).await;
    assert_eq!(first.body["entries"].as_array().unwrap().len(), 500);
    let cursor = first.body["next_cursor"].as_str().unwrap().to_string();
    let second = get(
        &app,
        &format!("/dictionary/entries?cursor={cursor}"),
        Caller::Proof,
    )
    .await;
    assert_eq!(second.body["entries"].as_array().unwrap().len(), 120);
    assert_eq!(second.body["next_cursor"], Value::Null);
    let mut all: BTreeSet<String> = written(&first.body).into_iter().collect();
    all.extend(written(&second.body));
    assert_eq!(all.len(), 620);
}

/// DD-38 on a keyed daemon: every write, and the export, needs the person's proof. A
/// stated private provider is standing for listing and bias, never authority.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn every_write_needs_the_proof() {
    let (_state, app) = daemon().await;
    let id = add(&app, "EPAS1", false).await;
    for caller in [Caller::SecretOnly, Caller::StatedPrivate] {
        for (method, uri, body) in [
            (
                "POST",
                "/dictionary/entries".to_string(),
                json!({"written": "UCSF"}),
            ),
            (
                "PATCH",
                format!("/dictionary/entries/{id}"),
                json!({"note": "x"}),
            ),
            ("DELETE", format!("/dictionary/entries/{id}"), Value::Null),
            (
                "POST",
                format!("/dictionary/entries/{id}/tier"),
                json!({"tier": "private"}),
            ),
            (
                "POST",
                "/dictionary/entries/delete".to_string(),
                json!({"ids": [id]}),
            ),
            (
                "POST",
                "/dictionary/entries/keep".to_string(),
                json!({"ids": [id]}),
            ),
            (
                "POST",
                "/dictionary/suggestions/accept".to_string(),
                json!({"ids": [id]}),
            ),
            (
                "POST",
                "/dictionary/suggestions/dismiss".to_string(),
                json!({"ids": [id]}),
            ),
            (
                "POST",
                "/dictionary/import".to_string(),
                json!({"format": "lines", "content": "UCSF"}),
            ),
            (
                "POST",
                "/dictionary/observations".to_string(),
                json!({"session_id": null, "observations": []}),
            ),
            (
                "POST",
                "/dictionary/settings".to_string(),
                json!({"agent_writes": "suggest"}),
            ),
            ("GET", "/dictionary/export".to_string(), Value::Null),
        ] {
            let body = (!body.is_null()).then_some(body);
            let reply = send(&app, method, &uri, caller, body, &[]).await;
            assert_eq!(
                reply.status,
                StatusCode::FORBIDDEN,
                "{method} {uri} {caller:?}"
            );
            assert_eq!(reply.body["code"], "agent", "{method} {uri}");
            assert_eq!(reply.body["message"], MSG_DICTIONARY_AGENT);
        }
    }
    assert_eq!(store_rows().len(), 1, "nothing was written");
    let settings = get(&app, "/dictionary/settings", Caller::SecretOnly).await;
    assert_eq!(settings.status, StatusCode::OK);
    assert_eq!(
        settings.body["authority"]["write"],
        json!({"allowed": false, "reason": MSG_DICTIONARY_AGENT})
    );
    let proven = get(&app, "/dictionary/settings", Caller::Proof).await;
    for control in ["write", "add_private", "tier_change", "raise_settings"] {
        assert_eq!(
            proven.body["authority"][control]["allowed"], true,
            "{control}"
        );
    }
    assert_eq!(proven.body["agent_writes"], "on_request");
    let clear = post(&app, "/dictionary/clear", Caller::Proof, json!({})).await;
    assert_eq!(
        clear.status,
        StatusCode::NOT_FOUND,
        "there is no clear route"
    );
}

/// Add, the same letters warning, an add of an existing row, edits with `If-Match`, a
/// duplicate spelling, delete with its tombstone, and bulk delete over listed ids.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn add_edit_and_delete() {
    let (_state, app) = daemon().await;
    let id = add(&app, "EPAS1", false).await;
    let again = post(
        &app,
        "/dictionary/entries",
        Caller::Proof,
        json!({"written": "EPAS1"}),
    )
    .await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.body["created"], false);
    assert_eq!(again.body["entry"]["id"], id);
    let twin = post(
        &app,
        "/dictionary/entries",
        Caller::Proof,
        json!({"written": "E-PAS-1", "spoken_forms": ["epas one"], "kind": "gene_symbol"}),
    )
    .await;
    assert_eq!(twin.status, StatusCode::CREATED);
    assert_eq!(
        twin.body["warning"],
        "A term with the same letters exists: EPAS1"
    );
    assert_eq!(twin.body["entry"]["match"], "exact");
    let refused = post(
        &app,
        "/dictionary/entries",
        Caller::Proof,
        json!({"written": "https://example.org"}),
    )
    .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused.body["code"], "url");
    let secret = post(
        &app,
        "/dictionary/entries",
        Caller::Proof,
        json!({"written": "AKIAIOSFODNN7EXAMPLE"}),
    )
    .await;
    assert_eq!(secret.status, StatusCode::UNPROCESSABLE_ENTITY);

    let uri = format!("/dictionary/entries/{id}");
    let stale = send(
        &app,
        "PATCH",
        &uri,
        Caller::Proof,
        Some(json!({"note": "a gene"})),
        &[("If-Match", "7".to_string())],
    )
    .await;
    assert_eq!(stale.status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(stale.body["code"], "stale");
    assert_eq!(stale.body["rev"], 1);
    let edited = send(
        &app,
        "PATCH",
        &uri,
        Caller::Proof,
        Some(json!({"note": "a gene", "spoken_forms": ["e pass one"]})),
        &[("If-Match", "\"1\"".to_string())],
    )
    .await;
    assert_eq!(edited.status, StatusCode::OK, "{}", edited.body);
    assert_eq!(edited.body["entry"]["rev"], 2);
    assert_eq!(edited.body["entry"]["note"], "a gene");
    let cleared = send(
        &app,
        "PATCH",
        &uri,
        Caller::Proof,
        Some(json!({"note": null})),
        &[],
    )
    .await;
    assert_eq!(cleared.body["entry"]["note"], Value::Null);
    let duplicate = send(
        &app,
        "PATCH",
        &uri,
        Caller::Proof,
        Some(json!({"written": "E-PAS-1"})),
        &[],
    )
    .await;
    assert_eq!(duplicate.status, StatusCode::CONFLICT);
    assert_eq!(duplicate.body["code"], "duplicate");

    let deleted = send(&app, "DELETE", &uri, Caller::Proof, None, &[]).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let gone = send(&app, "DELETE", &uri, Caller::Proof, None, &[]).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    assert_eq!(gone.body["message"], store::MSG_NOT_FOUND);
    let view = dictionary_store::open_store().view().unwrap();
    assert!(view.file().tombstoned("epas1", EntryTier::Public));

    let ucsf = add(&app, "UCSF", false).await;
    let made_up = store::EntryId::random().to_string();
    let bulk = post(
        &app,
        "/dictionary/entries/delete",
        Caller::Proof,
        json!({"ids": [ucsf, made_up, "not-an-id"]}),
    )
    .await;
    assert_eq!(bulk.body, json!({"deleted": 1}));
    assert_eq!(
        written(&get(&app, "/dictionary/entries", Caller::Proof).await.body),
        ["E-PAS-1"]
    );
}

/// Keep reviews an agent row; accept activates suggestions (all or nothing at the cap);
/// dismiss deletes them with tombstones.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn keep_accept_and_dismiss() {
    let (_state, app) = daemon().await;
    let store = dictionary_store::open_store();
    let added = |term: NewTerm| match store
        .write(|file| file.add(term, Reach::All, store::schema::now()))
        .unwrap()
    {
        store::AddOutcome::Created { id, .. } => id.to_string(),
        other => panic!("{other:?}"),
    };
    let agent = added(NewTerm {
        source: EntrySource::Agent,
        ..NewTerm::person("EGLN1", EntryTier::Public)
    });
    let suggested = |w: &str| NewTerm {
        source: EntrySource::Correction,
        status: EntryStatus::Suggested,
        ..NewTerm::person(w, EntryTier::Public)
    };
    let spoke = added(suggested("SPOKE"));
    let vhl = added(suggested("VHL"));

    let kept = post(
        &app,
        "/dictionary/entries/keep",
        Caller::Proof,
        json!({"ids": [agent, spoke]}),
    )
    .await;
    assert_eq!(kept.body, json!({"kept": 1}));
    assert!(row("EGLN1", EntryTier::Public).unwrap().reviewed);
    let accepted = post(
        &app,
        "/dictionary/suggestions/accept",
        Caller::Proof,
        json!({"ids": [spoke]}),
    )
    .await;
    assert_eq!(accepted.body, json!({"accepted": 1}));
    let spoke_row = row("SPOKE", EntryTier::Public).unwrap();
    assert_eq!(
        (spoke_row.status, spoke_row.reviewed),
        (EntryStatus::Active, true)
    );
    let dismissed = post(
        &app,
        "/dictionary/suggestions/dismiss",
        Caller::Proof,
        json!({"ids": [vhl, spoke]}),
    )
    .await;
    assert_eq!(
        dismissed.body,
        json!({"dismissed": 1}),
        "an active row is not dismissed"
    );
    assert!(row("VHL", EntryTier::Public).is_none());
    assert!(store
        .view()
        .unwrap()
        .file()
        .tombstoned("vhl", EntryTier::Public));
}

/// Import plans and merges over listed rows and never deletes; export carries no ids or
/// usage, and private rows only on request with private standing, as an attachment.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn import_and_export() {
    let (_state, app) = daemon().await;
    add(&app, "EPAS1", false).await;
    let content = "EPAS1 = epas one\nUCSF\nhttps://x.org\n";
    let plan = post(
        &app,
        "/dictionary/import",
        Caller::Proof,
        json!({"format": "lines", "content": content, "dry_run": true}),
    )
    .await;
    assert_eq!(plan.status, StatusCode::OK, "{}", plan.body);
    assert_eq!(
        plan.body,
        json!({"new": 1, "merged": 1, "duplicate": 0, "invalid": [{"line": 3, "code": "url"}]})
    );
    assert_eq!(store_rows().len(), 1, "a dry run writes nothing");
    let done = post(
        &app,
        "/dictionary/import",
        Caller::Proof,
        json!({"format": "lines", "content": content}),
    )
    .await;
    assert_eq!(done.body, plan.body);
    assert_eq!(
        row("EPAS1", EntryTier::Public).unwrap().spoken_forms,
        ["epas one"]
    );
    let private = post(
        &app,
        "/dictionary/import",
        Caller::Proof,
        json!({"format": "csv", "content": "written,spoken_forms,note\nPHD2,,a gene\n", "private": true}),
    )
    .await;
    assert_eq!(private.body["new"], 1);
    assert_eq!(row("PHD2", EntryTier::Private).unwrap().affiliation, None);
    let not_export = post(
        &app,
        "/dictionary/import",
        Caller::Proof,
        json!({"format": "json", "content": "{\"format\": \"other\"}"}),
    )
    .await;
    assert_eq!(not_export.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(not_export.body["code"], "not_an_export");
    let huge = "A\n".repeat(dictionary::MAX_IMPORT_BYTES / 2 + 1);
    let too_large = post(
        &app,
        "/dictionary/import",
        Caller::Proof,
        json!({"format": "lines", "content": huge}),
    )
    .await;
    assert_eq!(too_large.status, StatusCode::PAYLOAD_TOO_LARGE);

    let public = get(&app, "/dictionary/export", Caller::Proof).await;
    assert_eq!(public.status, StatusCode::OK);
    assert_eq!(
        public.headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"biorouter-dictionary.json\""
    );
    let rows: Vec<&str> = public.body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["written"].as_str().unwrap())
        .collect();
    assert_eq!(rows, ["EPAS1", "UCSF"]);
    let text = public.body.to_string();
    for field in ["\"id\"", "\"uses\"", "suggested_in", "dct_"] {
        assert!(!text.contains(field), "{field} in {text}");
    }
    let with_private = get(
        &app,
        "/dictionary/export?include_private=true",
        Caller::Proof,
    )
    .await;
    let private_row = with_private.body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["written"] == "PHD2")
        .cloned()
        .unwrap();
    assert_eq!(private_row["tier"], "private");
    assert_eq!(private_row["affiliation"], Value::Null);
}

/// The tier change with the proof: publicize merges into the public twin, privatize
/// moves; the answer is the row that holds the term afterwards.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_tier_change_moves_or_merges() {
    let (_state, app) = daemon().await;
    let public = add(&app, "EPAS1", false).await;
    let private = add(&app, "EPAS1", true).await;
    let merged = post(
        &app,
        &format!("/dictionary/entries/{private}/tier"),
        Caller::Proof,
        json!({"tier": "public"}),
    )
    .await;
    assert_eq!(merged.status, StatusCode::OK, "{}", merged.body);
    assert_eq!(merged.body["entry"]["id"], public);
    assert_eq!(
        merged.body["entry"]["provenance"]["reason"],
        "publicized_by_user"
    );
    assert!(row("EPAS1", EntryTier::Private).is_none());
    let moved = post(
        &app,
        &format!("/dictionary/entries/{public}/tier"),
        Caller::Proof,
        json!({"tier": "private"}),
    )
    .await;
    assert_eq!(moved.body["entry"]["tier"], "private");
    assert_eq!(
        moved.body["entry"]["affiliation"],
        Value::Null,
        "the person's act with no chat (O-36)"
    );
    let missing = post(
        &app,
        &format!("/dictionary/entries/{}/tier", store::EntryId::random()),
        Caller::Proof,
        json!({"tier": "public"}),
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

/// Settings: raising and lowering with the proof, and the answer after the change.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn settings_change_with_the_proof() {
    let (_state, app) = daemon().await;
    let raised = post(
        &app,
        "/dictionary/settings",
        Caller::Proof,
        json!({"agent_writes": "add", "learn_from_chats": true}),
    )
    .await;
    assert_eq!(raised.status, StatusCode::OK, "{}", raised.body);
    assert_eq!(raised.body["agent_writes"], "add");
    assert_eq!(raised.body["learn_from_chats"], true);
    let lowered = post(
        &app,
        "/dictionary/settings",
        Caller::Proof,
        json!({"agent_writes": "suggest"}),
    )
    .await;
    assert_eq!(lowered.body["agent_writes"], "suggest");
}

fn versa() -> HashMap<String, String> {
    HashMap::from([
        ("BIOROUTER_PROVIDER".to_string(), "versa_azure".to_string()),
        (
            "BIOROUTER_MODEL".to_string(),
            VERSA_AZURE_DEFAULT_MODEL.to_string(),
        ),
        (
            "VERSA_AZURE_API_KEY".to_string(),
            "placeholder-never-sent".to_string(),
        ),
    ])
}

async fn chat(state: &Arc<AppState>, private: bool) -> String {
    let manager = state.session_manager();
    let chat = manager
        .create_session(
            PathBuf::from("/tmp/dictionary_routes"),
            "dictionary stamp (test fixture)".to_string(),
            SessionType::User,
        )
        .await
        .unwrap();
    if private {
        manager
            .update(&chat.id)
            .provider_name("versa_azure")
            .model_config(ModelConfig::new("gpt-4o").unwrap())
            .raise_privacy(SessionClassification::Private, "turn:versa_azure")
            .apply()
            .await
            .unwrap();
    }
    chat.id
}

/// Bind a live agent to `chat` on `provider`: versa (UCSF's model) or a loopback ollama
/// (a local model). Built, never asked anything.
async fn bind(state: &Arc<AppState>, chat: &str, provider: &str) {
    let agent = state.get_agent(chat.to_string()).await.unwrap();
    let built = match provider {
        "versa" => {
            with_config_overrides(versa(), async {
                biorouter::providers::create(
                    "versa_azure",
                    ModelConfig::new(VERSA_AZURE_DEFAULT_MODEL).unwrap(),
                )
                .await
            })
            .await
        }
        _ => {
            let local = HashMap::from([("OLLAMA_HOST".to_string(), "127.0.0.1".to_string())]);
            with_config_overrides(local, async {
                biorouter::providers::create("ollama", ModelConfig::new("qwen3.8:27b").unwrap())
                    .await
            })
            .await
        }
    }
    .expect("a provider");
    assert!(built.tier().is_private());
    agent.update_provider(built, chat).await.unwrap();
}

/// Write `value` straight into a chat's `session_affiliations` column, the way
/// `malformed_source_affiliation_fails_before_copying_a_session` writes `not-json`.
async fn write_affiliations(chat: &str, value: &str) {
    let path = SessionManager::shared_store_root()
        .join(SESSIONS_FOLDER)
        .join(DB_NAME);
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&path);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::query("UPDATE sessions SET session_affiliations = ?1 WHERE id = ?2")
        .bind(value)
        .bind(chat)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

fn institutions(entry: &Value) -> Value {
    entry["affiliation"].clone()
}

/// O-35, O-36: what a private add records from the chat it started in.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_private_add_takes_the_institutions_of_its_chat() {
    let (state, app) = daemon().await;
    let add_from = |written: &'static str, origin: Value| {
        let app = app.clone();
        async move {
            let mut body = json!({"written": written, "private": true});
            body.as_object_mut()
                .unwrap()
                .extend(origin.as_object().unwrap().clone());
            let reply = post(&app, "/dictionary/entries", Caller::Proof, body).await;
            assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
            reply.body["entry"].clone()
        }
    };
    // No origin: the person's add with no chat (O-36).
    assert_eq!(
        institutions(&add_from("NOORIGIN", json!({})).await),
        Value::Null
    );
    // A transcript the renderer could not place.
    assert_eq!(
        institutions(&add_from("UNRESOLVED", json!({"origin_unresolved": true})).await),
        json!({"institutions": []})
    );
    // A chat with no live agent: its model cannot be read.
    let idle = chat(&state, false).await;
    assert_eq!(
        institutions(&add_from("IDLECHAT", json!({"origin_session_id": idle})).await),
        json!({"institutions": []})
    );
    // A chat on a local model whose record holds UCSF's connector: {ucsf}.
    let local = chat(&state, false).await;
    bind(&state, &local, "ollama").await;
    state
        .session_manager()
        .record_session_affiliation(&local, InstitutionId::new("ucsf"))
        .await
        .unwrap();
    let ucsf = add_from("LOCALUCSF", json!({"origin_session_id": local})).await;
    assert_eq!(ucsf["affiliation"]["institutions"][0]["id"], "ucsf");
    assert_eq!(
        row("LOCALUCSF", EntryTier::Private).unwrap().affiliation,
        Some(RowAffiliation {
            institutions: vec!["ucsf".into()]
        })
    );
    // A record that cannot be read: unknown.
    write_affiliations(&local, "not-json").await;
    assert_eq!(
        institutions(&add_from("NOTJSON", json!({"origin_session_id": local})).await),
        json!({"institutions": []})
    );
    // A public add carries none, whatever its origin.
    let public = post(
        &app,
        "/dictionary/entries",
        Caller::Proof,
        json!({"written": "PUBLICADD", "origin_session_id": local}),
    )
    .await;
    assert_eq!(public.body["entry"]["affiliation"], Value::Null);
    state.clear_cached_agents().await;
    for id in [idle, local] {
        state.session_manager().delete_session(&id).await.unwrap();
    }
}

async fn observe(app: &Router, session_id: Option<&str>, observations: Value) -> StatusCode {
    post(
        app,
        "/dictionary/observations",
        Caller::Proof,
        json!({"session_id": session_id, "observations": observations}),
    )
    .await
    .status
}

fn correction(record: &str, from: &str, to: &str) -> Value {
    json!({"record_id": record, "kind": "correction", "from": from, "to": to})
}

/// DD-58 at the route: a correction in a chat bound to a private model whose row is
/// still public lands in a private row and leaves the public one alone; a resend counts
/// once; secret and PII pairs are dropped; corrected_to rises in the usage file.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn observations_land_by_the_chats_stamp_and_count_once() {
    let (state, app) = daemon().await;
    let public = add(&app, "EPAS1", false).await;
    let bound = chat(&state, false).await;
    bind(&state, &bound, "versa").await;
    let before = row("EPAS1", EntryTier::Public).unwrap();

    let pair = correction("dct_stream_a", "a pass one", "EPAS1");
    assert_eq!(
        observe(&app, Some(&bound), json!([pair])).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        row("EPAS1", EntryTier::Public).unwrap(),
        before,
        "the public row"
    );
    let twin = row("EPAS1", EntryTier::Private).expect("a private row");
    assert_eq!(twin.status, EntryStatus::Active);
    assert_eq!(twin.misrecognitions[0].text, "a pass one");
    assert_eq!(twin.misrecognitions[0].count, 1);
    assert_eq!(
        twin.affiliation,
        Some(RowAffiliation {
            institutions: vec!["ucsf".into()]
        }),
        "the chat's model is UCSF's"
    );
    // A resend: nothing counts twice.
    observe(&app, Some(&bound), json!([pair])).await;
    assert_eq!(row("EPAS1", EntryTier::Private).unwrap(), twin);
    // A second dictation's sighting activates it.
    observe(
        &app,
        Some(&bound),
        json!([correction("dct_stream_b", "a pass one", "EPAS1")]),
    )
    .await;
    assert_eq!(
        row("EPAS1", EntryTier::Private).unwrap().misrecognitions[0].count,
        2
    );
    let listed = get(&app, "/dictionary/entries", Caller::Proof).await;
    let private_view = listed.body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["tier"] == "private")
        .cloned()
        .unwrap();
    assert_eq!(private_view["corrected_to"], 2);
    assert_eq!(
        private_view["suggested_in"],
        bound.as_str(),
        "the chat it came from"
    );
    assert_eq!(
        private_view["came_from_a_chat"], true,
        "MSG-PUBLICIZE-WARN, not the no chat sentence"
    );
    let public_view = listed.body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == public.as_str())
        .cloned()
        .unwrap();
    assert_eq!(
        public_view["corrected_to"], 0,
        "a private stamp bumps no public row"
    );

    // Secret and PII pairs are dropped before the store is touched.
    let bytes = std::fs::read(dictionary_store::open_store().store_path()).unwrap();
    observe(
        &app,
        Some(&bound),
        json!([
            correction("dct_stream_c", "my key", "AKIAIOSFODNN7EXAMPLE"),
            correction("dct_stream_d", "call me", "555-867-5309"),
            correction("dct_stream_e", "the record", "MRN 12345678"),
        ]),
    )
    .await;
    assert_eq!(
        std::fs::read(dictionary_store::open_store().store_path()).unwrap(),
        bytes
    );
    // The route's own secret scan, alone: a key shape the store's validator lets through
    // (no digit, so not its base64 rule) and the PII detector does not flag, which only
    // `secret_output::redact_text` stops.
    let key = format!("AIzaSy{}abcdefg", ('A'..='Z').collect::<String>());
    assert!(
        store::validate::term(&key).is_ok(),
        "the validator admits it"
    );
    assert!(!biorouter::guardrails::pii::PiiDetector::new().has_pii(&key));
    assert!(biorouter::guardrails::secret_output::redact_text(&key).is_some());
    observe(
        &app,
        Some(&bound),
        json!([correction("dct_stream_f", "my maps key", &key)]),
    )
    .await;
    assert_eq!(
        std::fs::read(dictionary_store::open_store().store_path()).unwrap(),
        bytes,
        "a pair only the secret scan flags is dropped"
    );
    // At most 16 pairs.
    let many: Vec<Value> = (0..17)
        .map(|i| correction(&format!("r{i}"), "x", "UCSF"))
        .collect();
    assert_eq!(
        observe(&app, Some(&bound), json!(many)).await,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    state.clear_cached_agents().await;
    state
        .session_manager()
        .delete_session(&bound)
        .await
        .unwrap();
}

/// P-16, O-35: a private row another institution's chat wrote is left as it was by an
/// observation from a chat on UCSF's model; rows an observation creates record the chat's
/// institutions, `[]` when the record cannot be read, the chat has no live agent, or the
/// request names no chat.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn observations_stamp_institutions_and_respect_them() {
    let (state, app) = daemon().await;
    let store = dictionary_store::open_store();
    store
        .write(|file| {
            file.add(
                NewTerm {
                    affiliation: Some(RowAffiliation {
                        institutions: vec!["stanford".into()],
                    }),
                    ..NewTerm::person("STANFORDTERM", EntryTier::Private)
                },
                Reach::All,
                store::schema::now(),
            )
        })
        .unwrap();
    let stanford_before = row("STANFORDTERM", EntryTier::Private).unwrap();
    let ucsf = chat(&state, true).await;
    bind(&state, &ucsf, "versa").await;
    observe(
        &app,
        Some(&ucsf),
        json!([correction("r1", "stanford term", "STANFORDTERM")]),
    )
    .await;
    assert_eq!(
        row("STANFORDTERM", EntryTier::Private).unwrap(),
        stanford_before,
        "a row whose institutions do not admit UCSF's model"
    );
    // The control: the same chat's observations do land, with UCSF's institution.
    observe(
        &app,
        Some(&ucsf),
        json!([correction("r1b", "arnt", "ARNT")]),
    )
    .await;
    assert_eq!(
        row("ARNT", EntryTier::Private).unwrap().affiliation,
        Some(RowAffiliation {
            institutions: vec!["ucsf".into()]
        })
    );

    // A local model whose record holds UCSF's connector: a new row records {ucsf}.
    let local = chat(&state, true).await;
    bind(&state, &local, "ollama").await;
    state
        .session_manager()
        .record_session_affiliation(&local, InstitutionId::new("ucsf"))
        .await
        .unwrap();
    observe(
        &app,
        Some(&local),
        json!([correction("r2", "spoke", "SPOKE")]),
    )
    .await;
    let spoke = row("SPOKE", EntryTier::Private).unwrap();
    assert_eq!(
        (spoke.status, spoke.source),
        (EntryStatus::Suggested, EntrySource::Correction)
    );
    assert_eq!(spoke.suggested_in.as_deref(), Some(local.as_str()));
    assert_eq!(
        spoke.affiliation,
        Some(RowAffiliation {
            institutions: vec!["ucsf".into()]
        })
    );
    write_affiliations(&local, "not-json").await;
    observe(
        &app,
        Some(&local),
        json!([correction("r3", "v h l", "VHL")]),
    )
    .await;
    assert_eq!(
        row("VHL", EntryTier::Private).unwrap().affiliation,
        Some(RowAffiliation::unknown())
    );
    // A private chat with no live agent: the model cannot be read.
    let idle = chat(&state, true).await;
    observe(
        &app,
        Some(&idle),
        json!([correction("r4", "egg lin", "EGLN1")]),
    )
    .await;
    assert_eq!(
        row("EGLN1", EntryTier::Private).unwrap().affiliation,
        Some(RowAffiliation::unknown())
    );
    // No chat at all, from the person: private, with the unknown institutions.
    observe(&app, None, json!([correction("r5", "hiff", "HIF1A")])).await;
    assert_eq!(
        row("HIF1A", EntryTier::Private).unwrap().affiliation,
        Some(RowAffiliation::unknown())
    );
    // `EntryView.affiliation` appears on private rows only.
    add(&app, "PUBLICROW", false).await;
    let listed = get(&app, "/dictionary/entries", Caller::Proof).await;
    for entry in listed.body["entries"].as_array().unwrap() {
        // Every private row here was written from a chat or with none, so each carries
        // a value; the public one carries `null`.
        let private = entry["tier"] == "private";
        assert_eq!(!entry["affiliation"].is_null(), private, "{entry}");
    }
    state.clear_cached_agents().await;
    for id in [ucsf, local, idle] {
        state.session_manager().delete_session(&id).await.unwrap();
    }
}

/// "Who stamps the tier": a stamp fails toward Private. A chat stored public whose live
/// agent's model is not read (here no provider is bound, so the read fails; a Crew grant,
/// a failed Crew manager or the 250 ms bound end the read the same way) may be bound to a
/// private model before its first turn, so its observation lands private with the unknown
/// institutions, as does an add that started in it. The control: the same chat with no
/// live agent at all is stamped by its stored tier, public.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_live_chat_whose_model_is_not_read_stamps_private() {
    let (state, app) = daemon().await;
    let idle = chat(&state, false).await;
    observe(&app, Some(&idle), json!([correction("u1", "arnt", "ARNT")])).await;
    assert!(
        row("ARNT", EntryTier::Public).is_some(),
        "no agent: stored tier"
    );
    assert!(row("ARNT", EntryTier::Private).is_none());

    let unread = chat(&state, false).await;
    let agent = state.get_agent(unread.clone()).await.unwrap();
    assert!(
        agent.provider().await.is_err(),
        "the precondition: a live agent whose model cannot be read"
    );
    observe(
        &app,
        Some(&unread),
        json!([correction("u2", "spoke", "SPOKE")]),
    )
    .await;
    assert!(row("SPOKE", EntryTier::Public).is_none(), "never public");
    assert_eq!(
        row("SPOKE", EntryTier::Private).unwrap().affiliation,
        Some(RowAffiliation::unknown()),
        "the model is unknown"
    );
    let added = post(
        &app,
        "/dictionary/entries",
        Caller::Proof,
        json!({"written": "UNREADORIGIN", "private": true, "origin_session_id": unread}),
    )
    .await;
    assert_eq!(added.status, StatusCode::CREATED, "{}", added.body);
    assert_eq!(
        institutions(&added.body["entry"]),
        json!({"institutions": []})
    );
    state.clear_cached_agents().await;
    for id in [idle, unread] {
        state.session_manager().delete_session(&id).await.unwrap();
    }
}

#[cfg(unix)]
fn root() -> PathBuf {
    PathBuf::from(std::env::var("BIOROUTER_PATH_ROOT").expect("the sandbox root"))
}

/// A managed policy staged where the daemon reads it under this binary's sandbox root,
/// with the modes the trust check requires, and removed when dropped, so a failed
/// assertion never leaves it in force for the next test.
#[cfg(unix)]
struct StagedPolicy(PathBuf);

#[cfg(unix)]
impl StagedPolicy {
    fn new(yaml: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let dir = root().join("managed");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let file = dir.join("managed-policy.yaml");
        std::fs::write(&file, yaml).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        Self(file)
    }
}

#[cfg(unix)]
impl Drop for StagedPolicy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Seed a suggestion straight into the store; its id.
#[cfg(unix)]
fn suggestion(written: &str) -> String {
    let term = NewTerm {
        source: EntrySource::Correction,
        status: EntryStatus::Suggested,
        ..NewTerm::person(written, EntryTier::Public)
    };
    match dictionary_store::open_store()
        .write(|file| file.add(term, Reach::All, store::schema::now()))
        .unwrap()
    {
        store::AddOutcome::Created { id, .. } => id.to_string(),
        other => panic!("{other:?}"),
    }
}

/// Managed `dictionary: false`: writes and observations refused with MSG-POLICY, delete,
/// the bulk delete, dismiss and export allowed, and the authority says so before any
/// click. Unix only, for the file modes the policy's trust check reads.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn managed_policy_refuses_writes_but_not_deletes_or_export() {
    let (_state, app) = daemon().await;
    let id = add(&app, "EPAS1", false).await;
    let bulk_id = add(&app, "PHD2", false).await;
    let suggested = suggestion("SPOKE");
    let _policy = StagedPolicy::new("dictation:\n  dictionary: false\n");
    let reason = biorouter::managed::ManagedPolicy::load()
        .dictation()
        .dictionary
        .reason()
        .map(str::to_string)
        .expect("the staged policy is read");

    for (uri, body) in [
        ("/dictionary/entries", json!({"written": "UCSF"})),
        (
            "/dictionary/observations",
            json!({"session_id": null, "observations": [correction("r", "x", "UCSF")]}),
        ),
        (
            "/dictionary/import",
            json!({"format": "lines", "content": "UCSF"}),
        ),
        ("/dictionary/settings", json!({"learn_from_chats": true})),
    ] {
        let reply = post(&app, uri, Caller::Proof, body).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{uri}");
        assert_eq!(reply.body["code"], "policy");
        assert_eq!(reply.body["message"], reason.as_str());
    }
    let settings = get(&app, "/dictionary/settings", Caller::Proof).await;
    assert_eq!(
        settings.body["authority"]["write"],
        json!({"allowed": false, "reason": reason})
    );
    let export = get(&app, "/dictionary/export", Caller::Proof).await;
    assert_eq!(export.status, StatusCode::OK);
    let deleted = send(
        &app,
        "DELETE",
        &format!("/dictionary/entries/{id}"),
        Caller::Proof,
        None,
        &[],
    )
    .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let dismissed = post(
        &app,
        "/dictionary/suggestions/dismiss",
        Caller::Proof,
        json!({"ids": [suggested]}),
    )
    .await;
    assert_eq!(dismissed.status, StatusCode::OK, "{}", dismissed.body);
    assert_eq!(dismissed.body, json!({"dismissed": 1}));
    let bulk_deleted = post(
        &app,
        "/dictionary/entries/delete",
        Caller::Proof,
        json!({"ids": [bulk_id]}),
    )
    .await;
    assert_eq!(bulk_deleted.status, StatusCode::OK, "{}", bulk_deleted.body);
    assert_eq!(bulk_deleted.body, json!({"deleted": 1}));
    assert!(
        store_rows().is_empty(),
        "every row deleted under the policy"
    );
}

/// Managed `dictionary_agent_writes: false` is a choke point of `POST /dictionary/settings`:
/// raising `agent_writes` or turning chat learning on is refused with MSG-POLICY, even with
/// the proof, lowering is still the person's, and the authority says so before any click.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn managed_agent_writes_off_refuses_raising_the_settings() {
    let (_state, app) = daemon().await;
    let _policy = StagedPolicy::new("dictation:\n  dictionary_agent_writes: false\n");
    let policy = biorouter::managed::ManagedPolicy::load();
    assert!(
        policy.dictation().dictionary.is_allowed(),
        "only the agent's writes are off"
    );
    let reason = policy
        .dictation()
        .dictionary_agent_writes
        .reason()
        .map(str::to_string)
        .expect("the staged policy is read");
    for raise in [
        json!({"agent_writes": "add"}),
        json!({"learn_from_chats": true}),
    ] {
        let reply = post(&app, "/dictionary/settings", Caller::Proof, raise.clone()).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{raise}");
        assert_eq!(
            reply.body,
            json!({"code": "policy", "message": reason.as_str()})
        );
    }
    let lowered = post(
        &app,
        "/dictionary/settings",
        Caller::Proof,
        json!({"agent_writes": "suggest"}),
    )
    .await;
    assert_eq!(lowered.status, StatusCode::OK, "{}", lowered.body);
    assert_eq!(lowered.body["agent_writes"], "suggest");
    let authority = &lowered.body["authority"];
    assert_eq!(
        authority["raise_settings"],
        json!({"allowed": false, "reason": reason})
    );
    assert_eq!(authority["write"], json!({"allowed": true, "reason": null}));
}
