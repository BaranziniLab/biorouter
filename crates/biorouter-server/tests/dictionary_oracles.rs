//! The existence oracle rules of DD-40 (`docs/dictation/dictionary.md`, "Existence oracle
//! rules"), each as a byte identical test: what a caller without private standing sees
//! and gets back is the same whether or not the store holds private rows, private
//! tombstones, private usage or a full private tier.
//!
//! The daemon holds no user action key, as `biorouter serve` starts one, because that is
//! where a caller without private standing may write at all; on the desktop's daemon such
//! a caller is refused every write with one sentence (`tests/dictionary_routes.rs`). Rules
//! 6, 9 and 11 are about dictation and are asked of `bias_set` directly, with standing from
//! the same `HttpCaller` the routes use.
//!
//! Its own binary: nothing here installs a digest, and the user action digest is a process
//! global.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode};
use axum::Router;
use biorouter::dictation::{bias_set, BiasChannels, BiasTarget, DictionaryRequest, Standing};
use biorouter::security::dictionary_store::{self, store};
use biorouter_server::routes::dictionary;
use biorouter_server::routes::session_reach;
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use store::{
    Entry, EntryId, EntrySource, EntryStatus, EntryTier, MatchPolicy, NewTerm, Reach, StoreFile,
    UsageDelta, UsageScope,
};
use tower::ServiceExt;

const SECRET: &str = "dictionary-oracle-secret";

async fn daemon() -> Router {
    let _ = std::fs::remove_dir_all(dictionary_store::dictionary_dir());
    let state = AppState::new().await.expect("app state");
    dictionary::routes(Arc::clone(&state)).layer(axum::middleware::from_fn_with_state(
        SECRET.to_string(),
        biorouter_server::auth::check_token,
    ))
}

fn reset() {
    let _ = std::fs::remove_dir_all(dictionary_store::dictionary_dir());
}

#[derive(Clone, Copy, Debug)]
enum Caller {
    /// The daemon secret and nothing else: no private standing.
    Public,
    /// A private host's stated provider (SD-12): private standing.
    Private,
}

/// The raw answer: status, the headers that matter, and the body's bytes.
#[derive(Debug, PartialEq, Eq)]
struct Raw {
    status: StatusCode,
    etag: Option<String>,
    body: Vec<u8>,
}

async fn raw(app: &Router, method: &str, uri: &str, caller: Caller, body: Option<Value>) -> Raw {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Secret-Key", SECRET)
        .header("content-type", "application/json");
    if let Caller::Private = caller {
        builder = builder.header("X-Caller-Provider", "versa_azure");
    }
    let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let etag = response
        .headers()
        .get(header::ETAG)
        .map(|v| v.to_str().unwrap().to_string());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    Raw { status, etag, body }
}

/// An answer with every per write value (ids, revisions, times) masked, so two answers
/// that differ only by when and under which random id a row was written compare equal.
fn masked(raw: &Raw) -> (StatusCode, Value) {
    fn mask(value: &mut Value) {
        match value {
            Value::Object(map) => {
                for (key, field) in map.iter_mut() {
                    if ["id", "created_at", "updated_at", "next_cursor"].contains(&key.as_str()) {
                        *field = json!("…");
                    } else {
                        mask(field);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(mask),
            _ => {}
        }
    }
    let mut value: Value = serde_json::from_slice(&raw.body).unwrap_or(Value::Null);
    mask(&mut value);
    (raw.status, value)
}

fn seed(term: NewTerm) -> EntryId {
    match dictionary_store::open_store()
        .write(|file| file.add(term, Reach::All, store::schema::now()))
        .unwrap()
    {
        store::AddOutcome::Created { id, .. } => id,
        other => panic!("{other:?}"),
    }
}

fn seed_private(written: &str) -> EntryId {
    seed(NewTerm::person(written, EntryTier::Private))
}

fn store_bytes() -> Option<Vec<u8>> {
    std::fs::read(dictionary_store::open_store().store_path()).ok()
}

/// Run `probe` against two stores that differ only by what `private` adds to the second,
/// and answer both results.
async fn with_and_without<T>(
    app: &Router,
    public: impl Fn(),
    private: impl Fn(),
    probe: impl Fn(Router) -> std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>,
) -> (T, T) {
    reset();
    public();
    let without = probe(app.clone()).await;
    reset();
    public();
    private();
    let with = probe(app.clone()).await;
    (without, with)
}

/// Rule 1: a public caller's add consults public rows and public tombstones only; a
/// `private: true` add or import is refused with one constant sentence before any row is
/// read, the same bytes for an existing private term and an absent one.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn rule_1_a_public_add_consults_public_rows_and_tombstones_only() {
    let app = daemon().await;
    let (without, with) = with_and_without(
        &app,
        || {},
        || {
            seed_private("EPAS1");
            seed_private("Epas-1");
            let doomed = seed_private("PHD2");
            dictionary_store::open_store()
                .write(|file| file.remove(&doomed, store::schema::now()).map(|_| ()))
                .unwrap();
        },
        |app| {
            Box::pin(async move {
                let refused_add = raw(
                    &app,
                    "POST",
                    "/dictionary/entries",
                    Caller::Public,
                    Some(json!({"written": "EPAS1", "private": true})),
                )
                .await;
                let refused_import = raw(
                    &app,
                    "POST",
                    "/dictionary/import",
                    Caller::Public,
                    Some(json!({"format": "lines", "content": "EPAS1", "private": true})),
                )
                .await;
                let added = raw(
                    &app,
                    "POST",
                    "/dictionary/entries",
                    Caller::Public,
                    Some(json!({"written": "EPAS1"})),
                )
                .await;
                let tombstoned = raw(
                    &app,
                    "POST",
                    "/dictionary/entries",
                    Caller::Public,
                    Some(json!({"written": "PHD2"})),
                )
                .await;
                (
                    refused_add,
                    refused_import,
                    masked(&added),
                    masked(&tombstoned),
                )
            })
        },
    )
    .await;
    assert_eq!(without.0, with.0, "the private add refusal");
    assert_eq!(without.0.status, StatusCode::FORBIDDEN);
    assert_eq!(without.1, with.1, "the private import refusal");
    assert_eq!(
        without.2, with.2,
        "a public add beside a private twin and its letters"
    );
    assert_eq!(without.2 .0, StatusCode::CREATED);
    assert!(without.2 .1.get("warning").is_none());
    assert_eq!(without.3, with.3, "a public add beside a private tombstone");
}

/// Rule 2: ids are random, and an id the caller may not list answers exactly as a
/// missing one, on every route that names an id.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn rule_2_an_unlisted_id_answers_as_a_missing_one() {
    let app = daemon().await;
    let private = seed_private("EPAS1").to_string();
    let made_up = EntryId::random().to_string();
    for (method, path, body) in [
        ("PATCH", "", Some(json!({"note": "x"}))),
        ("DELETE", "", None),
        ("POST", "/tier", Some(json!({"tier": "public"}))),
    ] {
        let mut answers = Vec::new();
        for id in [&private, &made_up] {
            answers.push(
                raw(
                    &app,
                    method,
                    &format!("/dictionary/entries/{id}{path}"),
                    Caller::Public,
                    body.clone(),
                )
                .await,
            );
        }
        assert_eq!(answers[0], answers[1], "{method} {path}");
    }
    let not_found = raw(
        &app,
        "DELETE",
        &format!("/dictionary/entries/{made_up}"),
        Caller::Public,
        None,
    )
    .await;
    assert_eq!(not_found.status, StatusCode::NOT_FOUND);
    assert_eq!(
        serde_json::from_slice::<Value>(&not_found.body).unwrap()["message"],
        store::MSG_NOT_FOUND
    );
    // Bulk routes skip it as missing, with the same counts.
    for uri in [
        "/dictionary/entries/delete",
        "/dictionary/entries/keep",
        "/dictionary/suggestions/accept",
        "/dictionary/suggestions/dismiss",
    ] {
        let a = raw(
            &app,
            "POST",
            uri,
            Caller::Public,
            Some(json!({"ids": [private]})),
        )
        .await;
        let b = raw(
            &app,
            "POST",
            uri,
            Caller::Public,
            Some(json!({"ids": [made_up]})),
        )
        .await;
        assert_eq!(a, b, "{uri}");
    }
    assert!(dictionary_store::open_store()
        .view()
        .unwrap()
        .find(&EntryId::parse(&private).unwrap())
        .is_some());
}

/// Rule 3: import dry runs and the same letters warning count only rows the caller can
/// list; and such a caller's import never touches a private row.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn rule_3_import_and_the_same_letters_warning_count_listed_rows_only() {
    let app = daemon().await;
    let (without, with) = with_and_without(
        &app,
        || {
            seed(NewTerm::person("UCSF", EntryTier::Public));
        },
        || {
            seed_private("PHD2");
            seed_private("Hif-1a");
        },
        |app| {
            Box::pin(async move {
                let content = "PHD2 = p h d two\nUCSF\nHIF1A\n";
                let plan = raw(
                    &app,
                    "POST",
                    "/dictionary/import",
                    Caller::Public,
                    Some(json!({"format": "lines", "content": content, "dry_run": true})),
                )
                .await;
                let warned = raw(
                    &app,
                    "POST",
                    "/dictionary/entries",
                    Caller::Public,
                    Some(json!({"written": "HIF-1A"})),
                )
                .await;
                (plan, masked(&warned))
            })
        },
    )
    .await;
    assert_eq!(without.0, with.0, "the import plan");
    assert_eq!(
        serde_json::from_slice::<Value>(&with.0.body).unwrap(),
        json!({"new": 2, "merged": 0, "duplicate": 1, "invalid": []})
    );
    assert_eq!(
        without.1, with.1,
        "no same letters warning from a private row"
    );

    // A real import beside a private PHD2 leaves the private row byte for byte.
    reset();
    let private = seed_private("PHD2");
    let before = row_json(&private);
    raw(
        &app,
        "POST",
        "/dictionary/import",
        Caller::Public,
        Some(json!({"format": "lines", "content": "PHD2 = p h d two"})),
    )
    .await;
    assert_eq!(row_json(&private), before);
}

fn row_json(id: &EntryId) -> String {
    serde_json::to_string(
        dictionary_store::open_store()
            .view()
            .unwrap()
            .find(id)
            .unwrap(),
    )
    .unwrap()
}

/// Rule 4: no store wide revision is exposed; the listing's `ETag` hashes the caller's
/// own view, so a private write changes neither what a public caller polls nor its tag.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn rule_4_a_private_write_never_moves_a_public_listing() {
    let app = daemon().await;
    seed(NewTerm::person("EPAS1", EntryTier::Public));
    let before = raw(&app, "GET", "/dictionary/entries", Caller::Public, None).await;
    let settings_before = raw(&app, "GET", "/dictionary/settings", Caller::Public, None).await;
    seed_private("PHD2");
    let doomed = seed_private("EGLN1");
    dictionary_store::open_store()
        .write(|file| file.remove(&doomed, store::schema::now()).map(|_| ()))
        .unwrap();
    let after = raw(&app, "GET", "/dictionary/entries", Caller::Public, None).await;
    assert_eq!(before, after);
    assert!(before.etag.is_some());
    let settings_after = raw(&app, "GET", "/dictionary/settings", Caller::Public, None).await;
    assert_eq!(settings_before, settings_after);
    let text = String::from_utf8(after.body).unwrap();
    for word in ["revision", "generation", "total", "count"] {
        assert!(!text.contains(word), "{word}: {text}");
    }
}

/// Rule 5: `uses`, `last_used_at`, `corrected_to` and `suggested_in` go only to a caller
/// with private standing, because usage on a public row moves when private chats dictate.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn rule_5_usage_needs_private_standing() {
    let app = daemon().await;
    let id = seed(NewTerm {
        suggested_in: Some("20261106_7".into()),
        source: EntrySource::Correction,
        ..NewTerm::person("EPAS1", EntryTier::Public)
    });
    let before = raw(&app, "GET", "/dictionary/entries", Caller::Public, None).await;
    // A private chat's dictation used the public row: the `all` map moves.
    assert!(dictionary_store::open_store()
        .flush_usage(
            &[(
                id.clone(),
                UsageScope::All,
                UsageDelta {
                    uses: 3,
                    corrected_to: 1,
                    last_used_at: Some(store::schema::now()),
                },
            )],
            Duration::from_millis(500),
        )
        .unwrap());
    let after = raw(&app, "GET", "/dictionary/entries", Caller::Public, None).await;
    assert_eq!(before, after);
    let text = String::from_utf8(after.body).unwrap();
    for field in ["uses", "last_used_at", "corrected_to", "suggested_in"] {
        assert!(!text.contains(field), "{field}: {text}");
    }
    let standing = raw(&app, "GET", "/dictionary/entries", Caller::Private, None).await;
    let entry = &serde_json::from_slice::<Value>(&standing.body).unwrap()["entries"][0];
    assert_eq!(entry["uses"], 3);
    assert_eq!(entry["corrected_to"], 1);
    assert_eq!(entry["suggested_in"], "20261106_7");
}

async fn standing(private: bool) -> Standing {
    let mut headers = HeaderMap::new();
    if private {
        headers.insert("X-Caller-Provider", HeaderValue::from_static("versa_azure"));
    }
    session_reach::http_caller(&headers)
        .await
        .dictation_standing()
}

fn view() -> Arc<store::StoreView> {
    dictionary_store::open_store().view().unwrap()
}

fn terms(eligible: &biorouter::dictation::recognize::EligibleEntries) -> Vec<String> {
    let mut out: Vec<String> = eligible
        .entries()
        .iter()
        .map(|e| e.written.clone())
        .collect();
    out.sort();
    out
}

/// Rule 6: dictation uses private terms only with standing and a private target, and
/// the answer has the same shape either way (its scope says public).
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn rule_6_private_terms_need_standing_and_a_private_target() {
    reset();
    seed(NewTerm::person("EPAS1", EntryTier::Public));
    seed_private("PHD2");
    let private_target = BiasTarget {
        private: true,
        affiliation: None,
        person_included: false,
    };
    let public_standing = standing(false).await;
    let private_standing = standing(true).await;
    for (target, standing) in [
        (&private_target, public_standing),
        (&BiasTarget::public(), private_standing),
        (&BiasTarget::public(), public_standing),
    ] {
        let eligible = bias_set(&view(), target, standing, true);
        assert_eq!(terms(&eligible), ["EPAS1"]);
        assert_eq!(
            eligible.scope(),
            biorouter::dictation::recognize::BiasScope::Public
        );
    }
    let eligible = bias_set(&view(), &private_target, private_standing, true);
    assert_eq!(terms(&eligible), ["EPAS1", "PHD2"]);
}

/// Rule 7: refusals name no term.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn rule_7_refusals_name_no_term() {
    let app = daemon().await;
    let term = "Zyxoterm";
    let private = seed_private(term).to_string();
    let mut refusals = vec![
        raw(
            &app,
            "POST",
            "/dictionary/entries",
            Caller::Public,
            Some(json!({"written": term, "private": true})),
        )
        .await,
        raw(
            &app,
            "POST",
            &format!("/dictionary/entries/{private}/tier"),
            Caller::Private,
            Some(json!({"tier": "public"})),
        )
        .await,
        raw(
            &app,
            "PATCH",
            &format!("/dictionary/entries/{private}"),
            Caller::Public,
            Some(json!({"written": term})),
        )
        .await,
        raw(
            &app,
            "POST",
            "/dictionary/import",
            Caller::Public,
            Some(json!({"format": "lines", "content": term, "private": true})),
        )
        .await,
        raw(
            &app,
            "POST",
            "/dictionary/settings",
            Caller::Public,
            Some(json!({"agent_writes": "add"})),
        )
        .await,
    ];
    // A person's add at the public cap: the cap sentence, not the term.
    fill(EntryTier::Public, store::ops::MAX_ACTIVE_PER_TIER);
    refusals.push(
        raw(
            &app,
            "POST",
            "/dictionary/entries",
            Caller::Public,
            Some(json!({"written": term})),
        )
        .await,
    );
    for refusal in refusals {
        assert!(refusal.status.is_client_error(), "{refusal:?}");
        let text = String::from_utf8(refusal.body).unwrap();
        assert!(!text.contains(term), "{text}");
        assert!(!text.contains(&private), "{text}");
    }
}

/// A row written straight into the file, with every field the store's own writers set.
fn bare_row(written: String, tier: EntryTier) -> Entry {
    let at = store::schema::now();
    Entry {
        id: EntryId::random(),
        rev: 1,
        written,
        tier,
        status: EntryStatus::Active,
        source: EntrySource::User,
        reviewed: true,
        spoken_forms: vec![],
        misrecognitions: vec![],
        kind: None,
        match_policy: MatchPolicy::Exact,
        bias: store::BiasPolicy::Auto,
        context_cues: vec![],
        note: None,
        reason: None,
        suggested_in: None,
        seen: 1,
        created_at: at,
        updated_at: at,
        provenance: None,
        affiliation: None,
    }
}

/// `count` active rows of `tier`, written straight into the file.
fn fill(tier: EntryTier, count: usize) {
    let store = dictionary_store::open_store();
    store
        .write(|file| {
            for i in 0..count {
                file.entries
                    .push(bare_row(format!("FILL{i}{}", tier.as_str()), tier));
            }
            Ok(())
        })
        .unwrap();
}

/// A public suggestion the chat extractor wrote, seen `seen` times; its id.
fn seed_auto(written: &str, seen: u32) -> EntryId {
    let row = Entry {
        status: EntryStatus::Suggested,
        source: EntrySource::Auto,
        reviewed: false,
        suggested_in: Some("20261106_3".into()),
        seen,
        ..bare_row(written.to_string(), EntryTier::Public)
    };
    let id = row.id.clone();
    dictionary_store::open_store()
        .write(|file| {
            file.entries.push(row);
            Ok(())
        })
        .unwrap();
    id
}

/// "Routes": an `auto` row seen once is never listed by any route, to a caller with or
/// without private standing. The listing omits it, every `{id}` route answers its id
/// exactly as a made up one, and the bulk routes count it as missing. The control: the
/// same kind of row seen twice is listed, edited and accepted.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn an_auto_row_seen_once_is_never_listed() {
    let app = daemon().await;
    let once = seed_auto("SPOKE", 1);
    let twice = seed_auto("ARNT", 2);
    let before = store_bytes();
    let made_up = EntryId::random();
    for caller in [Caller::Public, Caller::Private] {
        let listing = raw(&app, "GET", "/dictionary/entries", caller, None).await;
        let value: Value = serde_json::from_slice(&listing.body).unwrap();
        let listed: Vec<&str> = value["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["written"].as_str().unwrap())
            .collect();
        assert_eq!(listed, ["ARNT"], "{caller:?}");
        for (method, path, body) in [
            ("PATCH", "", Some(json!({"note": "x"}))),
            ("DELETE", "", None),
            ("POST", "/tier", Some(json!({"tier": "private"}))),
        ] {
            let mut answers = Vec::new();
            for id in [&once, &made_up] {
                answers.push(
                    raw(
                        &app,
                        method,
                        &format!("/dictionary/entries/{id}{path}"),
                        caller,
                        body.clone(),
                    )
                    .await,
                );
            }
            assert_eq!(answers[0], answers[1], "{caller:?} {method} {path}");
            assert!(answers[0].status.is_client_error(), "{caller:?} {method}");
        }
        for (uri, counted) in [
            ("/dictionary/entries/delete", "deleted"),
            ("/dictionary/entries/keep", "kept"),
            ("/dictionary/suggestions/accept", "accepted"),
            ("/dictionary/suggestions/dismiss", "dismissed"),
        ] {
            let a = raw(&app, "POST", uri, caller, Some(json!({"ids": [once]}))).await;
            let b = raw(&app, "POST", uri, caller, Some(json!({"ids": [made_up]}))).await;
            assert_eq!(a, b, "{caller:?} {uri}");
            let answer = serde_json::from_slice::<Value>(&a.body).unwrap();
            assert_eq!(answer[counted], 0, "{caller:?} {uri}: {answer}");
        }
    }
    assert_eq!(store_bytes(), before, "nothing reached either row");

    let edited = raw(
        &app,
        "PATCH",
        &format!("/dictionary/entries/{twice}"),
        Caller::Public,
        Some(json!({"note": "a protein"})),
    )
    .await;
    assert_eq!(edited.status, StatusCode::OK);
    let accepted = raw(
        &app,
        "POST",
        "/dictionary/suggestions/accept",
        Caller::Public,
        Some(json!({"ids": [twice]})),
    )
    .await;
    assert_eq!(
        serde_json::from_slice::<Value>(&accepted.body).unwrap(),
        json!({"accepted": 1})
    );
}

/// Rule 8: caps count only the caller's tier. A full private tier changes nothing a
/// public caller's add gets back, and at the public cap the refusal is the same whether
/// the private tier is full or empty.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn rule_8_caps_count_only_the_callers_tier() {
    let app = daemon().await;
    let add = |app: Router| {
        Box::pin(async move {
            masked(
                &raw(
                    &app,
                    "POST",
                    "/dictionary/entries",
                    Caller::Public,
                    Some(json!({"written": "EPAS1"})),
                )
                .await,
            )
        })
            as std::pin::Pin<Box<dyn std::future::Future<Output = (StatusCode, Value)> + Send>>
    };
    let (without, with) = with_and_without(
        &app,
        || {},
        || fill(EntryTier::Private, store::ops::MAX_ACTIVE_PER_TIER),
        add,
    )
    .await;
    assert_eq!(without, with);
    assert_eq!(without.0, StatusCode::CREATED);
    let (without, with) = with_and_without(
        &app,
        || fill(EntryTier::Public, store::ops::MAX_ACTIVE_PER_TIER),
        || fill(EntryTier::Private, store::ops::MAX_ACTIVE_PER_TIER),
        add,
    )
    .await;
    assert_eq!(without, with);
    assert_eq!(without.0, StatusCode::CONFLICT);
    assert_eq!(without.1["code"], "cap");
}

/// Rule 9: the decoder bonus tables are rebuilt at every revision of the store, so a
/// public scope table is rebuilt by a private write too, and its timing does not say
/// which tier changed.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn rule_9_every_revision_rebuilds_every_scope() {
    reset();
    seed(NewTerm::person("EPAS1", EntryTier::Public));
    let public = standing(false).await;
    let first = bias_set(&view(), &BiasTarget::public(), public, true);
    seed_private("PHD2");
    let second = bias_set(&view(), &BiasTarget::public(), public, true);
    assert_eq!(terms(&first), terms(&second), "the same public rows");
    assert_ne!(
        first.revision(),
        second.revision(),
        "a private write rebuilds the public table"
    );
}

/// Rule 10: no response reports how many entries were eligible or withheld.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn rule_10_no_response_counts_what_it_withholds() {
    let app = daemon().await;
    seed(NewTerm::person("EPAS1", EntryTier::Public));
    seed_private("PHD2");
    let listing = raw(&app, "GET", "/dictionary/entries", Caller::Public, None).await;
    let value: Value = serde_json::from_slice(&listing.body).unwrap();
    let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["entries", "next_cursor"]);
    let settings = raw(&app, "GET", "/dictionary/settings", Caller::Public, None).await;
    let value: Value = serde_json::from_slice(&settings.body).unwrap();
    let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
    // PR-23 (DD-56) adds the activation choice, a switch and not a count.
    assert_eq!(
        keys,
        [
            "suggest_from_chats",
            "learn_from_chats",
            "agent_writes",
            "authority"
        ]
    );
}

/// Rule 11: ranking for a public scope reads only the `public` usage map, so its prompt
/// is identical whatever private scope dictations and corrections have happened. The
/// control: the same bump does reach a private scope prompt.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn rule_11_public_ranking_reads_only_public_usage() {
    reset();
    // Spellings no English word collides with, so every row may enter the prompt.
    let mut ids = Vec::new();
    for i in 0..40 {
        ids.push(seed(NewTerm::person(
            format!("QZV{i:02}"),
            EntryTier::Public,
        )));
    }
    let public = standing(false).await;
    let private = standing(true).await;
    let private_target = BiasTarget {
        private: true,
        affiliation: None,
        person_included: false,
    };
    let prompt = |target: &BiasTarget, standing: Standing| {
        DictionaryRequest::new(
            bias_set(&view(), target, standing, true),
            "",
            BiasChannels::DEFAULT,
        )
        .prompt_terms()
    };
    let before = prompt(&BiasTarget::public(), public);
    let private_before = prompt(&private_target, private);
    assert_eq!(before.len(), 30, "{before:?}");
    let outside = ids
        .iter()
        .zip(0..)
        .find(|(_, i)| {
            !before
                .iter()
                .any(|t| t.trim_end_matches('.') == format!("QZV{i:02}"))
        })
        .map(|(id, _)| id.clone())
        .expect("a row outside the prompt");
    assert!(dictionary_store::open_store()
        .flush_usage(
            &[(
                outside,
                UsageScope::All,
                UsageDelta {
                    uses: 500,
                    corrected_to: 50,
                    last_used_at: Some(store::schema::now()),
                },
            )],
            Duration::from_millis(500),
        )
        .unwrap());
    assert_eq!(prompt(&BiasTarget::public(), public), before);
    assert_ne!(
        prompt(&private_target, private),
        private_before,
        "the control: a private scope ranks by the `all` map"
    );
}

/// An observation that names no chat, from a caller without private standing, is a
/// silent no op: its stamp is private, which that caller cannot list, so the store stays
/// byte identical and the answer is 204.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn an_observation_naming_no_chat_from_a_public_caller_changes_nothing() {
    let app = daemon().await;
    seed(NewTerm::person("EPAS1", EntryTier::Public));
    seed_private("PHD2");
    let before = store_bytes();
    let usage_before = std::fs::read(dictionary_store::open_store().usage_path()).ok();
    let answer = raw(
        &app,
        "POST",
        "/dictionary/observations",
        Caller::Public,
        Some(json!({"session_id": null, "observations": [
            {"record_id": "dct_stream_1", "kind": "correction", "from": "a pass one", "to": "EPAS1"},
            {"record_id": "dct_stream_1", "kind": "correction", "from": "p h d two", "to": "PHD2"},
            {"record_id": "dct_stream_1", "kind": "correction", "from": "spoke", "to": "SPOKE"},
        ]})),
    )
    .await;
    assert_eq!(answer.status, StatusCode::NO_CONTENT);
    assert!(answer.body.is_empty());
    assert_eq!(store_bytes(), before);
    assert_eq!(
        std::fs::read(dictionary_store::open_store().usage_path()).ok(),
        usage_before
    );
    // The same request naming a chat that does not exist: the same answer.
    let named = raw(
        &app,
        "POST",
        "/dictionary/observations",
        Caller::Public,
        Some(json!({"session_id": "20261106_no_such_chat", "observations": [
            {"record_id": "dct_stream_1", "kind": "correction", "from": "a pass one", "to": "EPAS1"},
        ]})),
    )
    .await;
    assert_eq!(named, answer);
    assert_eq!(store_bytes(), before);
    let _ = StoreFile::default();
}
