use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use biorouter::providers::api_client::{ApiClient, AuthMethod};
use serde_json::{json, Value};
use tracing::instrument::WithSubscriber;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const SYNTHETIC_KEY: &str = "synthetic-redirect-test-key";

/// The debug-log capture below installs a thread-local subscriber, and
/// tracing caches each callsite's interest process-wide from whichever thread
/// reaches it first. Every test here drives the same client, so without these
/// dispatchers a sibling's request that is first to reach any event on that
/// path, while the capture is the only subscriber registered, caches `never`
/// for it — and a payload that event did carry would be dropped before the
/// capture saw it: the "never logs the prompt" assertions would pass without
/// looking. (Its positive control is a `debug!` only that test reaches, so it
/// would not notice.) Mechanism and measurement: `biorouter_mcp::test_tracing`.
#[ctor::ctor]
fn register_inert_tracing_dispatchers_before_main() {
    biorouter_mcp::test_tracing::register_inert_dispatchers();
}

#[test]
fn the_inert_tracing_dispatchers_are_registered_before_main() {
    assert!(
        biorouter_mcp::test_tracing::inert_dispatchers_registered(),
        "the ctor no longer registers the inert tracing dispatchers, so the debug-log \
         captures in this binary can miss a transport event another test reached first"
    );
}

fn isolated_environment(root: &Path) -> env_lock::EnvGuard<'static> {
    env_lock::lock_env([
        ("BIOROUTER_PATH_ROOT", Some(root.to_str().unwrap())),
        ("BIOROUTER_DISABLE_KEYRING", Some("true")),
        ("BIOROUTER_CLIENT_CERT_PATH", None),
        ("BIOROUTER_CLIENT_KEY_PATH", None),
        ("BIOROUTER_CA_CERT_PATH", None),
        ("HTTP_PROXY", None),
        ("HTTPS_PROXY", None),
        ("ALL_PROXY", None),
        ("http_proxy", None),
        ("https_proxy", None),
        ("all_proxy", None),
        ("NO_PROXY", Some("*")),
        ("no_proxy", Some("*")),
    ])
}

fn client(server: &MockServer, rebuild: bool) -> ApiClient {
    let client = ApiClient::with_timeout(
        server.uri(),
        AuthMethod::ApiKey {
            header_name: "api-key".into(),
            key: SYNTHETIC_KEY.into(),
        },
        Duration::from_secs(5),
    )
    .unwrap();
    if rebuild {
        client.with_header("x-synthetic-rebuild", "yes").unwrap()
    } else {
        client
    }
}

fn assert_synthetic_request(request: &Request, body: &Value, rebuild: bool) {
    assert_eq!(request.method.as_str(), "POST");
    assert_eq!(request.headers.get("api-key").unwrap(), SYNTHETIC_KEY);
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap(),
        *body
    );
    if rebuild {
        assert_eq!(request.headers.get("x-synthetic-rebuild").unwrap(), "yes");
    }
}

async fn same_origin_preserves_request(status: u16, rebuild: bool) {
    let root = tempfile::tempdir().unwrap();
    let _environment = isolated_environment(root.path());
    let server = MockServer::start().await;
    Mock::given(path("/start"))
        .respond_with(ResponseTemplate::new(status).insert_header("Location", "/finish"))
        .mount(&server)
        .await;
    Mock::given(path("/finish"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let body = json!({"prompt":"fictional inventory", "items":["A","B"]});
    let response = client(&server, rebuild)
        .response_post("/start", &body)
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert_synthetic_request(request, &body, rebuild);
    }
    assert_eq!(requests[1].url.path(), "/finish");
}

async fn cross_port_receives_nothing(status: u16, rebuild: bool) {
    let root = tempfile::tempdir().unwrap();
    let _environment = isolated_environment(root.path());
    let source = MockServer::start().await;
    let target = MockServer::start().await;
    Mock::given(path("/start"))
        .respond_with(
            ResponseTemplate::new(status)
                .insert_header("Location", format!("{}/capture", target.uri())),
        )
        .mount(&source)
        .await;
    Mock::given(path("/capture"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&target)
        .await;
    let body = json!({"prompt":"synthetic-only", "arguments":{"item":"A"}});
    let result = client(&source, rebuild)
        .response_post("/start", &body)
        .await;
    let source_requests = source.received_requests().await.unwrap();
    assert_eq!(source_requests.len(), 1);
    assert_synthetic_request(&source_requests[0], &body, rebuild);
    let arrivals = target.received_requests().await.unwrap();
    assert_eq!(
        arrivals.len(),
        0,
        "cross-origin destination received a request"
    );
    assert!(result.is_err(), "cross-origin redirects must fail closed");
}

#[tokio::test]
async fn api_client_redirect_same_origin_307_preserves_key_and_body() {
    same_origin_preserves_request(307, false).await;
}

#[tokio::test]
async fn api_client_redirect_same_origin_308_preserves_key_and_body() {
    same_origin_preserves_request(308, false).await;
}

#[tokio::test]
async fn api_client_redirect_cross_port_307_receives_no_key_or_body() {
    cross_port_receives_nothing(307, false).await;
}

#[tokio::test]
async fn api_client_redirect_cross_port_308_receives_no_key_or_body() {
    cross_port_receives_nothing(308, false).await;
}

#[tokio::test]
async fn api_client_redirect_cross_port_301_receives_no_key_after_method_rewrite() {
    for rebuild in [false, true] {
        cross_port_receives_nothing(301, rebuild).await;
    }
}

#[tokio::test]
async fn api_client_redirect_cross_port_302_receives_no_key_after_method_rewrite() {
    for rebuild in [false, true] {
        cross_port_receives_nothing(302, rebuild).await;
    }
}

#[tokio::test]
async fn api_client_redirect_cross_port_303_receives_no_key_after_method_rewrite() {
    for rebuild in [false, true] {
        cross_port_receives_nothing(303, rebuild).await;
    }
}

#[tokio::test]
async fn api_client_redirect_rebuilt_cross_port_307_receives_no_key_or_body() {
    cross_port_receives_nothing(307, true).await;
}

#[tokio::test]
async fn api_client_redirect_rebuilt_cross_port_308_receives_no_key_or_body() {
    cross_port_receives_nothing(308, true).await;
}

#[tokio::test]
async fn api_client_redirect_rebuilt_same_origin_preserves_key_and_body() {
    for status in [307, 308] {
        same_origin_preserves_request(status, true).await;
    }
}

async fn redirect_limit(redirects: usize, rebuild: bool) {
    let root = tempfile::tempdir().unwrap();
    let _environment = isolated_environment(root.path());
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(move |request: &Request| {
            let hop: usize = request.url.path().trim_start_matches('/').parse().unwrap();
            if hop < redirects {
                ResponseTemplate::new(307).insert_header("Location", format!("/{}", hop + 1))
            } else {
                ResponseTemplate::new(200)
            }
        })
        .mount(&server)
        .await;
    let result = client(&server, rebuild)
        .response_post("/0", &json!({"synthetic":true}))
        .await;
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 11);
    if redirects == 10 {
        assert_eq!(result.unwrap().status(), 200);
    } else {
        let error = result.unwrap_err();
        assert!(error
            .downcast_ref::<reqwest::Error>()
            .unwrap()
            .is_redirect());
        assert!(requests.iter().all(|request| request.url.path() != "/11"));
    }
}

#[tokio::test]
async fn api_client_redirect_allows_existing_ten_hop_limit() {
    for rebuild in [false, true] {
        redirect_limit(10, rebuild).await;
    }
}

#[tokio::test]
async fn api_client_redirect_rejects_eleventh_hop() {
    for rebuild in [false, true] {
        redirect_limit(11, rebuild).await;
    }
}

#[derive(Clone, Default)]
struct DebugCapture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for DebugCapture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn api_client_debug_log_does_not_record_request_payload_or_credentials() {
    const PROMPT: &str = "SYNTHETIC_PROMPT_MUST_NOT_REACH_TRANSPORT_LOG";
    const RESPONSE: &str = "SYNTHETIC_RESPONSE_MUST_NOT_REACH_TRANSPORT_LOG";
    let root = tempfile::tempdir().unwrap();
    let _environment = isolated_environment(root.path());
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(ResponseTemplate::new(200).set_body_string(RESPONSE))
        .mount(&server)
        .await;
    let api = client(&server, false);
    let payload = json!({"messages":[{"role":"user", "content":PROMPT}]});
    let capture = DebugCapture::default();
    let writer = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .without_time()
        .with_writer(move || writer.clone())
        .finish();
    async {
        tracing::debug!("synthetic debug capture is active");
        let response = api.response_post("/completion", &payload).await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.text().await.unwrap(), RESPONSE);
    }
    .with_subscriber(subscriber)
    .await;
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_synthetic_request(&requests[0], &payload, false);
    let log = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("synthetic debug capture is active"));
    assert!(
        !log.contains(PROMPT),
        "transport debug log exposed the prompt"
    );
    assert!(
        !log.contains(SYNTHETIC_KEY),
        "transport debug log exposed the API key"
    );
    assert!(
        !log.contains(RESPONSE),
        "transport debug log exposed the response"
    );
}
