use biorouter::config::{with_config_overrides, Config};
use biorouter::model::ModelConfig;
use biorouter::providers::api_client::{ApiClient, AuthMethod};
use biorouter::providers::base::Provider;
use biorouter::providers::errors::{ProviderError, ProviderErrorKind};
use biorouter::providers::ollama::OllamaProvider;
use serde_json::json;
use std::collections::HashMap;
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[ctor::ctor]
fn scratch_config() {
    if biorouter::config::paths::Paths::path_root_override().is_some() {
        return;
    }
    let root = tempfile::TempDir::new().unwrap();
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let _ = ROOT.set(root);
}

async fn provider(host: &str) -> OllamaProvider {
    with_config_overrides(
        HashMap::from([("OLLAMA_HOST".to_string(), json!(host).to_string())]),
        OllamaProvider::from_env(ModelConfig::new("model:latest").unwrap()),
    )
    .await
    .unwrap()
}

async fn healthy(server: &MockServer, capabilities: &[&str]) {
    Mock::given(method("GET"))
        .and(path("/api/version"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"version":"0.40.13"})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "models":[{"name":"model:latest", "size":123}]
        })))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "capabilities":capabilities,
            "model_info":{"general.architecture":"llama", "llama.context_length":40960}
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn configured_service_is_not_ready_until_ollama_answers_and_recovers_without_host_change() {
    let server = MockServer::start().await;
    let provider = provider(&server.uri()).await;
    let offline = provider.readiness(None).await;
    assert!(!offline.reachable);
    assert_eq!(offline.phase, "service");
    assert!(offline.error.unwrap().contains("HTTP 404"));

    healthy(&server, &["completion", "tools"]).await;
    let ready = provider.readiness(Some("model")).await;
    assert!(ready.reachable, "{ready:?}");
    assert_eq!(ready.base_url, offline.base_url);
    assert_eq!(ready.version.as_deref(), Some("0.40.13"));
    assert_eq!(ready.model_available, Some(true));
    assert_eq!(ready.supports_tools, Some(true));
    assert_eq!(ready.context_window, Some(40960));
    assert_eq!(ready.phase, "ready");
    assert!(ready.error.is_none());
    assert!(server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|request| { request.url.path() != "/api/chat" }));
}

#[tokio::test]
async fn reachable_service_missing_model_and_unsupported_tools_are_distinct() {
    let server = MockServer::start().await;
    healthy(&server, &["completion"]).await;
    let provider = provider(&server.uri()).await;
    let missing = provider.readiness(Some("absent:8b")).await;
    assert!(missing.reachable);
    assert_eq!(missing.model_available, Some(false));
    assert!(missing.error.unwrap().contains("not installed"));
    let unsupported = provider.readiness(Some("model:latest")).await;
    assert!(unsupported.reachable);
    assert_eq!(unsupported.model_available, Some(true));
    assert_eq!(unsupported.supports_tools, Some(false));
    assert!(unsupported.error.unwrap().contains("does not support"));
    assert_eq!(unsupported.phase, "model");
}

#[tokio::test]
async fn checking_again_refreshes_replaced_model_capabilities_and_context() {
    let server = MockServer::start().await;
    healthy(&server, &["completion", "tools"]).await;
    let existing_chat = provider(&server.uri()).await;
    let provider = provider(&server.uri()).await;
    let first = provider.readiness(Some("model:latest")).await;
    assert_eq!(first.supports_tools, Some(true));
    assert_eq!(first.context_window, Some(40960));
    assert_eq!(
        existing_chat.context_window_for("model:latest").await,
        Some(40960)
    );

    server.reset().await;
    healthy(&server, &["completion"]).await;
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "capabilities":["completion"],
            "model_info":{"general.architecture":"llama", "llama.context_length":8192}
        })))
        .with_priority(1)
        .mount(&server)
        .await;
    let refreshed = provider.readiness(Some("model:latest")).await;
    assert_eq!(refreshed.supports_tools, Some(false));
    assert_eq!(refreshed.context_window, Some(8192));
    assert_eq!(
        existing_chat.context_window_for("model:latest").await,
        Some(8192)
    );

    server.reset().await;
    healthy(&server, &["completion", "tools"]).await;
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .respond_with(ResponseTemplate::new(503))
        .with_priority(1)
        .mount(&server)
        .await;
    let unknown = provider.readiness(Some("model:latest")).await;
    assert!(unknown.reachable);
    assert_eq!(unknown.supports_tools, None);
    assert_eq!(unknown.context_window, None);
}

#[tokio::test]
async fn status_bearing_transport_errors_preserve_fatal_and_transient_classification() {
    let server = MockServer::start().await;
    let client = reqwest::Client::new();
    for status in [400, 401, 403, 404, 422, 503] {
        let route = format!("/{status}");
        Mock::given(method("GET"))
            .and(path(&route))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let error = client
            .get(format!("{}{route}", server.uri()))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap_err();
        let error = ProviderError::from(error);
        assert_eq!(
            biorouter::agents::mistakes::is_recoverable(&error),
            status == 503,
            "{error}"
        );
    }
    let invalid = ProviderError::from(client.get("http://").send().await.unwrap_err());
    assert_eq!(invalid.kind(), ProviderErrorKind::InvalidRequest);
    assert!(!biorouter::agents::mistakes::is_recoverable(&invalid));
}

#[test]
fn malformed_host_diagnostics_do_not_echo_credentials_or_query_values() {
    for host in [
        "ftp://user:private-password@localhost/?token=private-query",
        "http://user:private-password@localhost:bad/?token=private-query",
    ] {
        let error = biorouter::providers::ollama::resolve_ollama_base_url(host)
            .unwrap_err()
            .to_string();
        assert!(error.contains("Invalid Ollama host"));
        assert!(!error.contains("private-password"), "{error}");
        assert!(!error.contains("private-query"), "{error}");
    }
}

#[tokio::test]
async fn refused_service_names_the_same_endpoint_without_blame_or_credentials() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let provider = provider(&format!(
        "http://user:private-password@{address}/?token=private-query"
    ))
    .await;
    let readiness = provider.readiness(None).await;
    assert!(!readiness.reachable);
    let error = readiness.error.unwrap();
    assert!(error.contains("Cannot reach Ollama"), "{error}");
    assert!(error.contains("connection refused"), "{error}");
    assert!(error.contains(&address.to_string()), "{error}");
    assert!(error.contains("does not start Ollama"), "{error}");
    assert!(!error.contains("private-password"));
    assert!(!error.contains("private-query"));
    assert!(!error.contains("port is correct"));
    let models = provider
        .fetch_supported_models()
        .await
        .unwrap_err()
        .to_string();
    assert!(models.contains("Cannot reach Ollama"), "{models}");
    assert!(!models.contains("private-password"));
}

#[tokio::test]
async fn readiness_has_an_overall_budget_even_if_server_never_finishes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/version"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
        .mount(&server)
        .await;
    let provider = provider(&server.uri()).await;
    let start = std::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(8), provider.readiness(None))
        .await
        .expect("readiness must not inherit the 600-second inference idle bound");
    assert!(!result.reachable);
    assert!(result.error.unwrap().contains("5 seconds"));
    assert!(start.elapsed() >= Duration::from_secs(5));
}

#[tokio::test]
async fn dns_tls_and_http_rejections_keep_distinct_safe_diagnostics() {
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let dns = client
        .get("http://biorouter-dns-check.invalid/api/version")
        .send()
        .await
        .unwrap_err();
    let dns = ProviderError::from(dns);
    assert!(dns.to_string().contains("DNS resolution failed"), "{dns}");
    assert_eq!(dns.kind(), ProviderErrorKind::Network);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut connection, _) = listener.accept().await.unwrap();
        let mut hello = [0; 8192];
        let _ = connection.read(&mut hello).await.unwrap();
        connection
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
    });
    let tls = client
        .get(format!("https://{address}/api/version"))
        .send()
        .await
        .unwrap_err();
    server.await.unwrap();
    let tls = ProviderError::from(tls);
    assert!(
        tls.to_string()
            .contains("TLS certificate or handshake failed"),
        "{tls}"
    );

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/version"))
        .respond_with(ResponseTemplate::new(401).set_body_string("SYNTHETIC_PRIVATE_RESPONSE"))
        .mount(&server)
        .await;
    let status = provider(&server.uri()).await.readiness(None).await;
    assert!(!status.reachable);
    let error = status.error.unwrap();
    assert!(error.contains("HTTP 401"), "{error}");
    assert!(!error.contains("SYNTHETIC_PRIVATE_RESPONSE"));
}

#[tokio::test]
async fn loopback_proxy_policy_is_verified_in_an_isolated_process() {
    let proxy = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"via_proxy":true})))
        .mount(&proxy)
        .await;
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "proxy_environment_child",
            "--ignored",
            "--nocapture",
        ])
        .env("HTTP_PROXY", proxy.uri())
        .env("http_proxy", proxy.uri())
        .env("HTTPS_PROXY", proxy.uri())
        .env("https_proxy", proxy.uri())
        .env("ALL_PROXY", proxy.uri())
        .env("all_proxy", proxy.uri())
        .env("NO_PROXY", "")
        .env("no_proxy", "")
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = proxy.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        1,
        "loopback requests must not visit the proxy"
    );
    assert_eq!(
        requests[0].url.host_str(),
        Some("biorouter-proxy-check.invalid")
    );
}

#[tokio::test]
#[ignore = "runs only as an isolated child with explicit proxy environment"]
async fn proxy_environment_child() {
    let _ = Config::global();
    let server = MockServer::start().await;
    healthy(&server, &["completion", "tools"]).await;
    let port = server.address().port();
    for host in [
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
    ] {
        let provider = provider(&host).await;
        assert!(provider.readiness(None).await.reachable, "{host}");
        let client = ApiClient::new(host, AuthMethod::BearerToken(String::new()))
            .unwrap()
            .with_header("x-rebuilt", "yes")
            .unwrap();
        let response = client.api_get("api/version").await.unwrap();
        assert_eq!(response.payload.unwrap()["version"], "0.40.13");
    }
    let ipv6 = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
    let address = ipv6.local_addr().unwrap();
    let serve_ipv6 = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for _ in 0..2 {
            let (mut connection, _) = ipv6.accept().await.unwrap();
            let mut request = [0; 8192];
            let count = connection.read(&mut request).await.unwrap();
            let path = String::from_utf8_lossy(&request[..count]);
            let body = if path.starts_with("GET /api/version ") {
                "{\"version\":\"0.40.13\"}"
            } else {
                "{\"models\":[]}"
            };
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            connection.write_all(response.as_bytes()).await.unwrap();
        }
    });
    assert!(
        provider(&format!("http://{address}"))
            .await
            .readiness(None)
            .await
            .reachable
    );
    serve_ipv6.await.unwrap();
    let remote = ApiClient::new(
        "http://biorouter-proxy-check.invalid".to_string(),
        AuthMethod::BearerToken(String::new()),
    )
    .unwrap()
    .with_header("x-rebuilt", "yes")
    .unwrap();
    assert_eq!(
        remote
            .api_get("api/version")
            .await
            .unwrap()
            .payload
            .unwrap()["via_proxy"],
        true
    );
}

#[test]
fn exhausted_transport_budget_is_not_multiplied_by_the_agent() {
    let error = ProviderError::RetryExhausted {
        source: Box::new(ProviderError::RequestFailed(
            "connection refused".to_string(),
        )),
        attempts: 4,
    };
    assert_eq!(error.kind(), ProviderErrorKind::Network);
    assert!(!biorouter::agents::mistakes::is_recoverable(&error));
    assert!(error.to_string().contains("after 4 request attempts"));
}

#[tokio::test]
async fn meditation_knowledge_repetition_still_stops_the_fifth_identical_call() {
    use biorouter::conversation::message::Message;
    use biorouter::tool_inspection::{InspectionAction, ToolInspector};
    use biorouter::tool_monitor::RepetitionInspector;
    let requests = (1..=5)
        .map(|index| {
            let message = Message::assistant().with_tool_request(
                format!("synthetic-{index}"),
                Ok(rmcp::model::CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: "knowledge__kb_list_bases".into(),
                    arguments: Some(serde_json::Map::new()),
                }),
            );
            message.content[0].as_tool_request().unwrap().clone()
        })
        .collect::<Vec<_>>();
    let session = biorouter::session::Session {
        id: "synthetic-meditation-repetition".to_string(),
        ..Default::default()
    };
    let results = RepetitionInspector::new(Some(4))
        .inspect(
            &requests,
            &[],
            biorouter::config::BioRouterMode::Approve,
            &session,
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].tool_request_id, "synthetic-5");
    assert_eq!(results[0].action, InspectionAction::Deny);
}
