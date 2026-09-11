//! Issue #56, QA 2026-09-10 (SD-9): a `biorouter serve` daemon's own web
//! interface keeps the reach its operator's provider implies — on the listing
//! and knowledge-base surfaces, which were open to it before they were gated —
//! and a caller holding only the daemon secret does not.
//!
//! ⚠ **Its own test binary on purpose.** The operator standing and the
//! user-action digest are both process-global `OnceLock`s. Nothing here
//! installs a digest, which is exactly how `biorouter serve` starts its daemon
//! (`Stdio::null()`, SD-7), and the operator standing installed below must not
//! leak into any other binary's view of the gates.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real `sessions.db`.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use axum::{body::Body, http::Request, Router};
use biorouter::conversation::message::Message;
use biorouter::model::ModelConfig;
use biorouter::privacy::{ProviderTier, SessionClassification};
use biorouter::session::SessionType;
use biorouter_mcp::knowledge::service::KnowledgeService;
use biorouter_server::routes::session_reach::{KNOWLEDGE_BASE_REACH_NO_KEY, SESSION_REACH_NO_KEY};
use biorouter_server::state::AppState;
use std::sync::Arc;
use tower::ServiceExt;

/// The browser token `biorouter serve` would have minted for this launch.
const BROWSER_TOKEN: &str = "9f1c2e7a5b3d4c6e8f0a1b2c3d4e5f60";
const SENTINEL: &str = "sd9-served-operator-marker-not-real-data";

/// The operator configured a private provider — institution-hosted — so SD-1
/// pins every session this daemon runs to a private model.
fn install_private_operator() {
    biorouter_server::auth::install_served_operator(
        BROWSER_TOKEN.to_string(),
        ProviderTier::Private,
    );
}

fn served_document_cookie() -> String {
    format!("biorouter_session={BROWSER_TOKEN}")
}

async fn send(app: &Router, uri: &str, cookie: Option<&str>) -> (u16, String) {
    let mut builder = Request::builder().uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    let res = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status().as_u16();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The Knowledge view in the operator's browser still reads a private base in
/// full; a caller holding the same secret without the served document's cookie
/// — or with a cookie that is not it — is refused, in the keyless daemon's own
/// words, and is listed only the public base.
#[tokio::test]
async fn the_served_interface_keeps_the_operators_reach_on_knowledge_bases() {
    install_private_operator();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let svc = Arc::new(KnowledgeService::new(root.clone()));
    svc.create_base("omop", "OMOP", None).unwrap();
    svc.create_base("notes", "Notes", None).unwrap();
    let page = root.join("omop").join("knowledge").join("x.md");
    std::fs::create_dir_all(page.parent().unwrap()).unwrap();
    std::fs::write(&page, format!("# x\n\n{SENTINEL}\n")).unwrap();
    biorouter_mcp::knowledge::tier::raise_unlocked(&root, "omop", true).unwrap();
    let app = biorouter_server::routes::knowledge::router(svc);

    let cookie = served_document_cookie();
    let (status, body) = send(&app, "/bases/omop/page?path=knowledge/x.md", Some(&cookie)).await;
    assert_eq!(
        status, 200,
        "the operator's own browser lost its Knowledge view: {body}"
    );
    assert!(body.contains(SENTINEL));
    let (status, body) = send(&app, "/bases", Some(&cookie)).await;
    assert_eq!(status, 200);
    assert!(body.contains("omop") && body.contains("notes"), "{body}");

    for (label, cookie) in [
        ("no cookie", None),
        (
            "a cookie that is not the served document's",
            Some("biorouter_session=guessed"),
        ),
        (
            "a cookie under another name",
            Some("other_session=9f1c2e7a5b3d4c6e8f0a1b2c3d4e5f60"),
        ),
    ] {
        let (status, body) = send(&app, "/bases/omop/page?path=knowledge/x.md", cookie).await;
        assert_eq!(
            (status, body.as_str()),
            (403, KNOWLEDGE_BASE_REACH_NO_KEY),
            "{label}: a caller holding only the secret read a private base"
        );
        let (status, body) = send(&app, "/bases", cookie).await;
        assert_eq!(status, 200);
        assert!(
            body.contains("notes") && !body.contains("omop"),
            "{label}: listed a private base: {body}"
        );
    }
}

/// On chats, the operator standing preserves the History list — and nothing
/// else. The transcript gate refused this browser every private chat before
/// this change and still does, and so does every route that names a chat:
/// deleting one is never cheaper than reading it. Widening the transcript gate
/// for a serve operator is recorded as an open decision (SD-9), not taken.
#[tokio::test(flavor = "multi_thread")]
async fn the_served_interface_keeps_its_history_list_and_gains_nothing_else() {
    install_private_operator();
    let state = AppState::new().await.unwrap();
    let manager = state.session_manager();
    let chat = manager
        .create_session(
            std::path::PathBuf::from("/tmp/sd9_served_operator"),
            "SD-9 private (test fixture)".to_string(),
            SessionType::User,
        )
        .await
        .unwrap();
    manager
        .add_message(&chat.id, &Message::user().with_text(SENTINEL))
        .await
        .unwrap();
    manager
        .update(&chat.id)
        .provider_name("versa_azure")
        .model_config(ModelConfig::new("gpt-4o").unwrap())
        .raise_privacy(SessionClassification::Private, "turn:versa_azure")
        .apply()
        .await
        .unwrap();
    let app = biorouter_server::routes::configure(state.clone(), "sd9-secret".to_string());
    let cookie = served_document_cookie();

    let (status, body) = send(&app, "/sessions", Some(&cookie)).await;
    assert_eq!(status, 200);
    assert!(
        body.contains(&chat.id),
        "the operator's history lost a private chat"
    );
    let (status, body) = send(&app, "/sessions", None).await;
    assert_eq!(status, 200);
    assert!(
        !body.contains(&chat.id),
        "a secret-only caller was listed a private chat"
    );

    // The transcript: refused before this change, refused after — with the
    // cookie or without it.
    for cookie in [Some(cookie.as_str()), None] {
        let (status, body) = send(&app, &format!("/sessions/{}", chat.id), cookie).await;
        assert_eq!(
            (status, body.as_str()),
            (403, SESSION_REACH_NO_KEY),
            "{cookie:?}: the served-operator standing reached a private transcript"
        );
    }
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/sessions/{}", chat.id))
                .header("cookie", served_document_cookie())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        403,
        "a route that names a chat admitted what the transcript read refuses"
    );
    assert!(manager.get_session(&chat.id, false).await.is_ok());

    manager.delete_session(&chat.id).await.unwrap();
}
