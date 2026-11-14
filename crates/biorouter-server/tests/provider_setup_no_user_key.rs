//! SD-18 (owner ruling, 2026-11-14): on a `biorouter serve` daemon, which holds
//! no proof-of-user key (SD-7), the person in the served page chooses the host's
//! providers and default model from the browser, and a caller holding only the
//! daemon secret still cannot.
//!
//! What a request from the served page carries, and a secret-only caller (a
//! model with a shell that recovered the secret, H2) does not: the served
//! document's `HttpOnly` cookie, and the Fetch Metadata a browser attaches to a
//! script request from a same-origin page. The tests below send each half alone
//! and both together, and check that the person's choice, and only it, moves
//! SD-12's launch pin and SD-10's served-page standing.
//!
//! ⚠ **Its own test binary on purpose**, for the reason `new_chat_no_user_key.rs`
//! gives: the user-action digest and the served operator are process-global
//! `OnceLock`s. Nothing here installs a digest, which is exactly how
//! `biorouter serve` starts its daemon.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can write the developer's real `config.yaml`.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::Router;
use biorouter::config::Config;
use biorouter::privacy::ProviderTier;
use biorouter_server::auth::{
    install_served_operator, provider_choice_proof, served_operator_capability, user_action_proof,
    UserActionProof,
};
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tower::ServiceExt;

/// The browser token `biorouter serve` would have minted for this launch.
const BROWSER_TOKEN: &str = "5d2a7c9e1b3f4a6c8e0d2b4f6a8c0e1d";

/// A request shaped like one the served page's own script sends.
const SERVED_PAGE: &[(&str, &str)] = &[
    (
        "cookie",
        "biorouter_session=5d2a7c9e1b3f4a6c8e0d2b4f6a8c0e1d",
    ),
    ("sec-fetch-site", "same-origin"),
    ("sec-fetch-mode", "cors"),
    ("sec-fetch-dest", "empty"),
];

/// The cookie alone: what `curl` sends after reading the token, without a
/// browser's Fetch Metadata. Also what a page at a plain-HTTP network address
/// sends, where a browser attaches none.
const COOKIE_ONLY: &[(&str, &str)] = &[(
    "cookie",
    "biorouter_session=5d2a7c9e1b3f4a6c8e0d2b4f6a8c0e1d",
)];

/// Browser Fetch Metadata without the served document's cookie.
const METADATA_ONLY: &[(&str, &str)] = &[
    ("sec-fetch-site", "same-origin"),
    ("sec-fetch-mode", "cors"),
    ("sec-fetch-dest", "empty"),
];

/// A cross-site request carrying the cookie (which `SameSite=Strict` would not
/// send, but a forged one might): refused for its `Sec-Fetch-Site`.
const CROSS_SITE: &[(&str, &str)] = &[
    (
        "cookie",
        "biorouter_session=5d2a7c9e1b3f4a6c8e0d2b4f6a8c0e1d",
    ),
    ("sec-fetch-site", "cross-site"),
    ("sec-fetch-mode", "cors"),
    ("sec-fetch-dest", "empty"),
];

/// A top-level navigation (a link, a form post): not the page's script.
const NAVIGATION: &[(&str, &str)] = &[
    (
        "cookie",
        "biorouter_session=5d2a7c9e1b3f4a6c8e0d2b4f6a8c0e1d",
    ),
    ("sec-fetch-site", "same-origin"),
    ("sec-fetch-mode", "navigate"),
    ("sec-fetch-dest", "document"),
];

/// A wrong token in the cookie, with otherwise perfect browser headers.
const WRONG_COOKIE: &[(&str, &str)] = &[
    ("cookie", "biorouter_session=not-the-token"),
    ("sec-fetch-site", "same-origin"),
    ("sec-fetch-mode", "cors"),
    ("sec-fetch-dest", "empty"),
];

const SECRET_ONLY: &[(&str, &str)] = &[];

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
    }
    map
}

/// A `biorouter serve` daemon with nothing configured: no digest (keyless), a
/// browser token (so its own page can be told apart, SD-10), and a launch state
/// recorded from an empty configuration by a launcher that promised no key.
fn launch_an_unconfigured_serve_daemon() {
    let config = Config::global();
    for key in biorouter::privacy::CAPABILITY_CONFIG_KEYS {
        let _ = config.delete(key);
    }
    install_served_operator(BROWSER_TOKEN.to_string(), ProviderTier::Public);
    biorouter_server::auth::adopt_served_operator_capability(ProviderTier::Public);
    biorouter_server::launch::record_launch_state(false);
    assert_eq!(
        user_action_proof(&HeaderMap::new()),
        UserActionProof::NoKeyInstalled,
        "something in this binary installed a user-action digest, so these tests would be \
         measuring a desktop daemon rather than a `biorouter serve` one"
    );
    assert!(
        biorouter::privacy::privacy_tiers_enabled(),
        "privacy tiers are off, so no gate below would fire either way"
    );
    assert_eq!(
        biorouter_server::launch::capability_config_moved_since_launch(),
        None,
        "the launch state was not recorded from the configuration it just cleared"
    );
}

async fn app() -> Router {
    let state: Arc<AppState> = AppState::new().await.unwrap();
    biorouter_server::routes::config_management::routes(state)
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    pairs: &[(&str, &str)],
    body: Option<Value>,
) -> (StatusCode, String) {
    let mut builder = Request::builder().uri(uri).method(method);
    for (name, value) in pairs {
        builder = builder.header(*name, *value);
    }
    let request = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = tokio::time::timeout(
        Duration::from_secs(60),
        axum::body::to_bytes(response.into_body(), usize::MAX),
    )
    .await
    .expect("the response body did not finish within a minute")
    .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// `ollama` at a loopback host is a provider this daemon can construct with no
/// network at all, and it is Private (`self_hosted_tier`): exactly the choice
/// SD-12 must then honour for a new chat.
fn ollama_choice() -> Value {
    json!({ "provider": "ollama", "model": "stub-model" })
}

/// The proof, as each kind of caller earns it. Only the served page's own
/// script is a person here; everything else is answered as before SD-18.
#[test]
#[serial]
fn only_the_served_pages_script_is_a_person() {
    launch_an_unconfigured_serve_daemon();
    assert_eq!(
        provider_choice_proof(&headers(SERVED_PAGE)),
        UserActionProof::Proven
    );
    for (what, pairs) in [
        ("the secret alone", SECRET_ONLY),
        ("the cookie without browser metadata", COOKIE_ONLY),
        ("browser metadata without the cookie", METADATA_ONLY),
        ("a cross-site request", CROSS_SITE),
        ("a navigation", NAVIGATION),
        ("a wrong cookie", WRONG_COOKIE),
    ] {
        assert_eq!(
            provider_choice_proof(&headers(pairs)),
            UserActionProof::NoKeyInstalled,
            "{what} was admitted as the person in the served page"
        );
    }
    // And nothing else that asks for a person learned to accept it: the general
    // proof (approvals, declassify, reset) is unchanged for the same request.
    assert_eq!(
        user_action_proof(&headers(SERVED_PAGE)),
        UserActionProof::NoKeyInstalled,
        "SD-18 widened the general proof of a person, which it must not"
    );
}

/// `GET /config/provider_setup` answers the page's question with the same
/// predicate, and says why in a person's words when the answer is no.
#[tokio::test]
#[serial]
async fn the_page_is_told_whether_it_may_choose_a_provider() {
    launch_an_unconfigured_serve_daemon();
    let app = app().await;

    let (status, body) = send(&app, "GET", "/config/provider_setup", SERVED_PAGE, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let answer: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["allowed"], json!(true), "{body}");
    assert!(answer.get("reason").is_none(), "{body}");

    // The cookie at a plain-HTTP network address, where a browser attaches no
    // Fetch Metadata: told about the insecure address.
    let at_a_network_address: Vec<(&str, &str)> = COOKIE_ONLY
        .iter()
        .copied()
        .chain([("host", "192.168.1.20:18731")])
        .collect();
    let (_, body) = send(
        &app,
        "GET",
        "/config/provider_setup",
        &at_a_network_address,
        None,
    )
    .await;
    let answer: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["allowed"], json!(false), "{body}");
    assert!(
        answer["reason"].as_str().unwrap().contains("plain HTTP"),
        "a page with no Fetch Metadata at a network address is told about it: {body}"
    );

    // The same headers at a loopback address are not a browser page at all
    // (curl, or a model's shell): told so, not blamed on the address.
    for pairs in [COOKIE_ONLY, SECRET_ONLY] {
        let at_loopback: Vec<(&str, &str)> = pairs
            .iter()
            .copied()
            .chain([("host", "127.0.0.1:18731")])
            .collect();
        let (_, body) = send(&app, "GET", "/config/provider_setup", &at_loopback, None).await;
        let answer: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(answer["allowed"], json!(false), "{body}");
        assert!(
            answer["reason"]
                .as_str()
                .unwrap()
                .contains("did not come from the page"),
            "a non-browser caller at loopback is told it is not the page: {body}"
        );
    }

    let (_, body) = send(&app, "GET", "/config/provider_setup", METADATA_ONLY, None).await;
    let answer: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["allowed"], json!(false), "{body}");
    assert!(
        answer["reason"]
            .as_str()
            .unwrap()
            .contains("`biorouter serve` printed"),
        "a page without the cookie is sent back to the printed address: {body}"
    );
}

/// The whole first-run path, as the page runs it: a secret-only caller is
/// refused at every write; the person's write lands, moves SD-12's pin and
/// SD-10's standing to their choice, and a shell's edit afterwards is still
/// drift.
#[tokio::test]
#[serial]
async fn the_person_in_the_page_chooses_the_provider_and_nothing_else_does() {
    launch_an_unconfigured_serve_daemon();
    let app = app().await;

    // A secret-only caller: refused, nothing written, nothing adopted.
    let (status, body) = send(
        &app,
        "POST",
        "/config/set_provider",
        SECRET_ONLY,
        Some(ollama_choice()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    for pairs in [
        COOKIE_ONLY,
        METADATA_ONLY,
        CROSS_SITE,
        NAVIGATION,
        WRONG_COOKIE,
    ] {
        let (status, body) = send(
            &app,
            "POST",
            "/config/set_provider",
            pairs,
            Some(ollama_choice()),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{pairs:?}: {body}");
    }
    let (status, body) = send(
        &app,
        "POST",
        "/config/upsert",
        SECRET_ONLY,
        Some(
            json!({ "key": "OLLAMA_HOST", "value": "http://127.0.0.1:11434", "is_secret": false }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(Config::global().get_biorouter_provider().is_err());
    assert_eq!(
        biorouter_server::launch::pinned_value("BIOROUTER_PROVIDER"),
        None
    );

    // The person in the page: the host and the default land.
    let (status, body) = send(
        &app,
        "POST",
        "/config/upsert",
        SERVED_PAGE,
        Some(
            json!({ "key": "OLLAMA_HOST", "value": "http://127.0.0.1:11434", "is_secret": false }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = send(
        &app,
        "POST",
        "/config/set_provider",
        SERVED_PAGE,
        Some(ollama_choice()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(Config::global().get_biorouter_provider().unwrap(), "ollama");

    // SD-12's pin follows the person's choice, so a new chat on it is not
    // refused as drift...
    assert_eq!(
        biorouter_server::launch::capability_config_moved_since_launch(),
        None,
        "the person's choice reads as drift, so every new chat on it would be refused"
    );
    assert_eq!(
        biorouter_server::launch::pinned_value("BIOROUTER_PROVIDER").as_deref(),
        Some("ollama")
    );
    // ...and SD-10's standing follows its tier.
    assert_eq!(
        served_operator_capability(&headers(SERVED_PAGE)),
        biorouter::workflow::privacy::declared_provider_tier("ollama").await,
    );

    // A shell that edits `config.yaml` afterwards is still drift: the pin took
    // the value the route wrote, not whatever the file says now.
    Config::global()
        .set_param("BIOROUTER_PROVIDER", "llamacpp")
        .unwrap();
    assert_eq!(
        biorouter_server::launch::capability_config_moved_since_launch().as_deref(),
        Some("BIOROUTER_PROVIDER"),
        "a shell-written provider was adopted as though the person had chosen it"
    );
    Config::global()
        .set_param("BIOROUTER_PROVIDER", "ollama")
        .unwrap();
    assert_eq!(
        biorouter_server::launch::capability_config_moved_since_launch(),
        None
    );

    // A secret-only write of a capability key never moves the pin, even where
    // the gate is not consulted: the same key, written through the file.
    Config::global()
        .set_param("OLLAMA_HOST", "https://ollama.example")
        .unwrap();
    assert_eq!(
        biorouter_server::launch::capability_config_moved_since_launch().as_deref(),
        Some("OLLAMA_HOST")
    );
    Config::global()
        .set_param("OLLAMA_HOST", "http://127.0.0.1:11434")
        .unwrap();

    // The person may also take a key back out, and the pin follows that too.
    let (status, body) = send(
        &app,
        "POST",
        "/config/remove",
        SECRET_ONLY,
        Some(json!({ "key": "OLLAMA_HOST", "is_secret": false })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let (status, body) = send(
        &app,
        "POST",
        "/config/remove",
        SERVED_PAGE,
        Some(json!({ "key": "OLLAMA_HOST", "is_secret": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(biorouter_server::launch::pinned_value("OLLAMA_HOST"), None);
    assert_eq!(
        biorouter_server::launch::capability_config_moved_since_launch(),
        None
    );
}

/// Reading the non-private-model disclosure is part of choosing a model, so
/// the person in the page may acknowledge it; a secret-only caller may not.
#[tokio::test]
#[serial]
async fn the_person_in_the_page_may_acknowledge_the_disclosure() {
    launch_an_unconfigured_serve_daemon();
    let app = app().await;
    let (status, body) = send(&app, "POST", "/privacy/disclosure/ack", SECRET_ONLY, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = send(&app, "POST", "/privacy/disclosure/ack", COOKIE_ONLY, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = send(&app, "POST", "/privacy/disclosure/ack", SERVED_PAGE, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// A launcher that promised a key and did not deliver it is the desktop's
/// fault state, not a served page: the page's headers earn nothing there.
#[test]
#[serial]
fn a_desktop_daemon_missing_its_key_is_not_a_served_page() {
    launch_an_unconfigured_serve_daemon();
    biorouter_server::launch::record_launch_state(true);
    assert_eq!(
        provider_choice_proof(&headers(SERVED_PAGE)),
        UserActionProof::NoKeyInstalled
    );
    biorouter_server::launch::record_launch_state(false);
}
