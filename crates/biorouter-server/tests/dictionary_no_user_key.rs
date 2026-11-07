//! `/dictionary/*` on a daemon that holds no user action key, as `biorouter serve` starts
//! one (PR-16; SD-7, DD-38, O-4; `docs/dictation/dictionary.md`, "Who may do what" and
//! "Refusals"):
//!
//! * add, accept and edit are admitted for rows the caller can list, and stamped with the
//!   caller's standing: a caller with private standing (a private host's stated provider,
//!   SD-12) writes private rows, and its edit of a public row lands in the private twin;
//! * tier changes are refused with MSG-DICTIONARY-HOST-TIER before the id is read;
//!   raising `agent_writes` and turning `learn_from_chats` on are refused with
//!   MSG-DICTIONARY-HOST-SETTINGS, and turning them down or off is admitted;
//! * a `private: true` add or import without private standing is refused with
//!   MSG-DICTIONARY-HOST-PRIVATE;
//! * a private host's `X-Caller-Provider` lists private rows; the served page's cookie,
//!   whose only private input is the operator's tier (SD-10), lists them too but never
//!   stamps a write private and never sees the usage figures.
//!
//! ⚠ Its own binary on purpose: the user action digest and the served operator are
//! process globals, and nothing here installs a digest, which is how `serve` starts its
//! daemon. (`suggest_from_chats` and its `POST /dictionary/activation` arrive with the
//! `dictionary` capability, PR-23.)

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::Router;
use biorouter::privacy::ProviderTier;
use biorouter::security::dictionary_store::{self, store};
use biorouter_server::auth::{user_action_proof, UserActionProof};
use biorouter_server::routes::dictionary::{
    self, msg_dictionary_host_settings, msg_dictionary_host_tier, MSG_DICTIONARY_HOST_PRIVATE,
    MSG_DICTIONARY_HOST_SETTINGS,
};
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use store::{EntrySource, EntryStatus, EntryTier, NewTerm, Reach};
use tower::ServiceExt;

const SECRET: &str = "dictionary-keyless-secret";
/// The browser token `biorouter serve` would have minted for this launch.
const BROWSER_TOKEN: &str = "5c9e1d3f7b2a4e6c8d0f1a3b5c7e9d2f";

async fn daemon() -> Router {
    assert_eq!(
        user_action_proof(&HeaderMap::new()),
        UserActionProof::NoKeyInstalled,
        "something in this binary installed a user action digest"
    );
    let _ = std::fs::remove_dir_all(dictionary_store::dictionary_dir());
    let state = AppState::new().await.expect("app state");
    dictionary::routes(Arc::clone(&state)).layer(axum::middleware::from_fn_with_state(
        SECRET.to_string(),
        biorouter_server::auth::check_token,
    ))
}

#[derive(Clone, Copy, Debug)]
enum Caller {
    SecretOnly,
    /// `userActionHeaders()` on a `serve` page of a private host (SD-12).
    StatedPrivate,
    /// The served document's cookie and nothing else: the operator's tier only.
    ServedPage,
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    caller: Caller,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Secret-Key", SECRET)
        .header("content-type", "application/json");
    builder = match caller {
        Caller::SecretOnly => builder,
        Caller::StatedPrivate => builder.header("X-Caller-Provider", "versa_azure"),
        Caller::ServedPage => {
            builder.header("cookie", format!("biorouter_session={BROWSER_TOKEN}"))
        }
    };
    let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn post(app: &Router, uri: &str, caller: Caller, body: Value) -> (StatusCode, Value) {
    send(app, "POST", uri, caller, Some(body)).await
}

fn row(written: &str, tier: EntryTier) -> Option<store::Entry> {
    dictionary_store::open_store()
        .view()
        .unwrap()
        .entries()
        .iter()
        .find(|e| e.written == written && e.tier == tier)
        .cloned()
}

fn suggest(written: &str, tier: EntryTier) -> String {
    let term = NewTerm {
        source: EntrySource::Correction,
        status: EntryStatus::Suggested,
        ..NewTerm::person(written, tier)
    };
    match dictionary_store::open_store()
        .write(|file| file.add(term, Reach::All, store::schema::now()))
        .unwrap()
    {
        store::AddOutcome::Created { id, .. } => id.to_string(),
        other => panic!("{other:?}"),
    }
}

/// O-4: a `serve` browser writes rows it can list, stamped with its standing.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn writes_are_admitted_for_listable_rows_and_stamped_with_standing() {
    let app = daemon().await;
    let (status, public) = post(
        &app,
        "/dictionary/entries",
        Caller::SecretOnly,
        json!({"written": "EPAS1"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{public}");
    assert_eq!(public["entry"]["tier"], "public");
    // A private host: every term its browser adds is private, `private` or not.
    let (status, private) = post(
        &app,
        "/dictionary/entries",
        Caller::StatedPrivate,
        json!({"written": "PHD2"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(private["entry"]["tier"], "private");
    assert_eq!(private["entry"]["affiliation"], Value::Null, "no origin");
    let (status, refused) = post(
        &app,
        "/dictionary/entries",
        Caller::SecretOnly,
        json!({"written": "EGLN1", "private": true}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(refused["code"], "host_private");
    assert_eq!(refused["message"], MSG_DICTIONARY_HOST_PRIVATE);
    assert!(row("EGLN1", EntryTier::Private).is_none());

    // Accept: a listable suggestion is accepted; a private one is skipped as missing.
    let public_suggestion = suggest("SPOKE", EntryTier::Public);
    let private_suggestion = suggest("VHL", EntryTier::Private);
    let (_, accepted) = post(
        &app,
        "/dictionary/suggestions/accept",
        Caller::SecretOnly,
        json!({"ids": [public_suggestion, private_suggestion]}),
    )
    .await;
    assert_eq!(accepted, json!({"accepted": 1}));
    assert_eq!(
        row("VHL", EntryTier::Private).unwrap().status,
        EntryStatus::Suggested
    );
    // With private standing the private suggestion is listed, and accepted.
    let (status, accepted) = post(
        &app,
        "/dictionary/suggestions/accept",
        Caller::StatedPrivate,
        json!({"ids": [private_suggestion]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert_eq!(accepted, json!({"accepted": 1}));
    let vhl = row("VHL", EntryTier::Private).unwrap();
    assert_eq!((vhl.status, vhl.reviewed), (EntryStatus::Active, true));

    // Edit: the public caller edits the public row in place; the private standing
    // caller's edit of it lands in the private twin, and the public row stays.
    let id = public["entry"]["id"].as_str().unwrap();
    let uri = format!("/dictionary/entries/{id}");
    let (status, edited) = send(
        &app,
        "PATCH",
        &uri,
        Caller::SecretOnly,
        Some(json!({"note": "a gene"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["entry"]["tier"], "public");
    let public_row = row("EPAS1", EntryTier::Public).unwrap();
    let (status, twin) = send(
        &app,
        "PATCH",
        &uri,
        Caller::StatedPrivate,
        Some(json!({"spoken_forms": ["epas one"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{twin}");
    assert_eq!(twin["entry"]["tier"], "private");
    assert_ne!(twin["entry"]["id"], id);
    assert_eq!(
        twin["entry"]["note"], "a gene",
        "the twin starts from the public row"
    );
    assert_eq!(row("EPAS1", EntryTier::Public).unwrap(), public_row);
    assert_eq!(
        row("EPAS1", EntryTier::Private).unwrap().affiliation,
        None,
        "an edit on the page carries no chat"
    );
    // A private row is not the public caller's to edit, and reads as missing.
    let private_id = private["entry"]["id"].as_str().unwrap();
    let (status, _) = send(
        &app,
        "PATCH",
        &format!("/dictionary/entries/{private_id}"),
        Caller::SecretOnly,
        Some(json!({"note": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(
        &app,
        "DELETE",
        &format!("/dictionary/entries/{private_id}"),
        Caller::SecretOnly,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(row("PHD2", EntryTier::Private).is_some());
}

/// DD-38: tier changes are the host's, refused before the id is read; raising the
/// settings is refused with the option named, and lowering is admitted.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn tier_changes_and_raised_settings_are_the_hosts() {
    let app = daemon().await;
    let (_, added) = post(
        &app,
        "/dictionary/entries",
        Caller::SecretOnly,
        json!({"written": "EPAS1"}),
    )
    .await;
    let id = added["entry"]["id"].as_str().unwrap().to_string();
    let made_up = store::EntryId::random().to_string();
    for caller in [Caller::SecretOnly, Caller::StatedPrivate] {
        for target in [&id, &made_up] {
            for (tier, want) in [
                ("private", msg_dictionary_host_tier(EntryTier::Private)),
                ("public", msg_dictionary_host_tier(EntryTier::Public)),
            ] {
                let (status, body) = post(
                    &app,
                    &format!("/dictionary/entries/{target}/tier"),
                    caller,
                    json!({"tier": tier}),
                )
                .await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {tier}");
                assert_eq!(body, json!({"code": "host_tier", "message": want}));
            }
        }
    }
    assert_eq!(row("EPAS1", EntryTier::Public).unwrap().rev, 1);

    for (raise, option) in [
        (json!({"agent_writes": "add"}), "--agent-writes add"),
        (json!({"learn_from_chats": true}), "--learn-from-chats on"),
    ] {
        let (status, body) = post(&app, "/dictionary/settings", Caller::StatedPrivate, raise).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            body,
            json!({"code": "host_settings", "message": msg_dictionary_host_settings(option)})
        );
    }
    // Turning chat learning off from on: the person turned it on at the terminal, which a
    // keyless caller cannot, and turning it off is admitted.
    dictionary_store::open_store()
        .write(|file| {
            file.settings.learn_from_chats = true;
            Ok(())
        })
        .unwrap();
    let (status, off) = post(
        &app,
        "/dictionary/settings",
        Caller::SecretOnly,
        json!({"learn_from_chats": false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{off}");
    assert_eq!(off["learn_from_chats"], false);
    assert!(
        !dictionary_store::open_store()
            .view()
            .unwrap()
            .settings()
            .learn_from_chats
    );
    let (status, lowered) = post(
        &app,
        "/dictionary/settings",
        Caller::SecretOnly,
        json!({"agent_writes": "suggest", "learn_from_chats": false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{lowered}");
    assert_eq!(lowered["agent_writes"], "suggest");
    let (status, _) = post(
        &app,
        "/dictionary/settings",
        Caller::SecretOnly,
        json!({"agent_writes": "suggest"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "naming the current value raises nothing"
    );

    let (_, settings) = send(
        &app,
        "GET",
        "/dictionary/settings",
        Caller::SecretOnly,
        None,
    )
    .await;
    let authority = &settings["authority"];
    assert_eq!(authority["write"], json!({"allowed": true, "reason": null}));
    assert_eq!(
        authority["add_private"],
        json!({"allowed": false, "reason": MSG_DICTIONARY_HOST_PRIVATE})
    );
    assert_eq!(
        authority["tier_change"],
        json!({"allowed": false, "reason": msg_dictionary_host_tier(EntryTier::Public)})
    );
    assert_eq!(
        authority["raise_settings"],
        json!({"allowed": false, "reason": MSG_DICTIONARY_HOST_SETTINGS})
    );
    let (_, stated) = send(
        &app,
        "GET",
        "/dictionary/settings",
        Caller::StatedPrivate,
        None,
    )
    .await;
    assert_eq!(stated["authority"]["add_private"]["allowed"], true);
}

/// A private import without private standing is refused before any row is read, both
/// for `private: true` and for a row the file marks private.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_private_import_needs_private_standing() {
    let app = daemon().await;
    let export = json!({
        "format": "biorouter.dictionary", "version": 1, "exported_at": "2026-11-06T00:00:00Z",
        "entries": [{"written": "PHD2", "spoken_forms": [], "misrecognitions": [], "kind": null,
                     "match": "exact", "note": null, "tier": "private", "affiliation": null}]
    })
    .to_string();
    for body in [
        json!({"format": "lines", "content": "UCSF", "private": true}),
        json!({"format": "json", "content": export}),
    ] {
        let (status, refused) = post(&app, "/dictionary/import", Caller::SecretOnly, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(refused["message"], MSG_DICTIONARY_HOST_PRIVATE);
    }
    assert!(
        !dictionary_store::dictionary_dir().exists(),
        "nothing read or written"
    );
    let (status, imported) = post(
        &app,
        "/dictionary/import",
        Caller::StatedPrivate,
        json!({"format": "json", "content": export}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{imported}");
    assert_eq!(imported["new"], 1);
    let (_, public) = post(
        &app,
        "/dictionary/import",
        Caller::SecretOnly,
        json!({"format": "lines", "content": "UCSF"}),
    )
    .await;
    assert_eq!(public["new"], 1);
    assert!(row("UCSF", EntryTier::Public).is_some());
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

/// SD-12 and SD-10: a private host's stated provider lists private rows and sees usage;
/// the served page's cookie alone lists them (the operator's tier) but never stamps a
/// write private and never sees usage; the secret alone lists public rows.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn standing_is_the_stated_provider_and_the_operators_tier_only_lists() {
    biorouter_server::auth::install_served_operator(
        BROWSER_TOKEN.to_string(),
        ProviderTier::Private,
    );
    let app = daemon().await;
    post(
        &app,
        "/dictionary/entries",
        Caller::SecretOnly,
        json!({"written": "EPAS1"}),
    )
    .await;
    post(
        &app,
        "/dictionary/entries",
        Caller::StatedPrivate,
        json!({"written": "PHD2"}),
    )
    .await;

    let (_, stated) = send(
        &app,
        "GET",
        "/dictionary/entries",
        Caller::StatedPrivate,
        None,
    )
    .await;
    assert_eq!(written(&stated), ["EPAS1", "PHD2"]);
    assert!(stated["entries"][0].get("uses").is_some());
    let (_, served) = send(&app, "GET", "/dictionary/entries", Caller::ServedPage, None).await;
    assert_eq!(
        written(&served),
        ["EPAS1", "PHD2"],
        "the operator's tier lists"
    );
    assert!(
        served["entries"][0].get("uses").is_none(),
        "but is not standing"
    );
    let (_, secret) = send(&app, "GET", "/dictionary/entries", Caller::SecretOnly, None).await;
    assert_eq!(written(&secret), ["EPAS1"]);

    let (status, by_page) = post(
        &app,
        "/dictionary/entries",
        Caller::ServedPage,
        json!({"written": "EGLN1"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(by_page["entry"]["tier"], "public", "never stamped private");
    let (status, _) = post(
        &app,
        "/dictionary/entries",
        Caller::ServedPage,
        json!({"written": "VHL", "private": true}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (_, export) = send(
        &app,
        "GET",
        "/dictionary/export?include_private=true",
        Caller::SecretOnly,
        None,
    )
    .await;
    assert!(
        !export.to_string().contains("PHD2"),
        "no standing, no private rows"
    );
    let (_, export) = send(
        &app,
        "GET",
        "/dictionary/export?include_private=true",
        Caller::StatedPrivate,
        None,
    )
    .await;
    assert!(export.to_string().contains("PHD2"));
}
