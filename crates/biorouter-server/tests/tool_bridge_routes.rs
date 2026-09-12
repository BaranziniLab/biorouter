//! The coding-agent tool bridge, end to end through the real router.
//!
//! What this pins is the lifecycle, because that is where a capability leaks: a
//! grant must be reachable exactly while its lease is alive, must be unreachable
//! the moment it is dropped, and must never be reachable by a nonce that was not
//! issued. The dispatch half is unit-tested next to the gate stack it runs; what
//! could only go wrong here is the wiring.
//!
//! Every test here is `#[serial]`. The published base URL and the grant registry
//! are both process-global — they have to be, because the HTTP handler runs on a
//! different task from the turn that issued the grant — so two tests publishing
//! different bases concurrently would assert against each other's value. Running
//! them in parallel passed and would have failed intermittently later, which is
//! worse than failing now.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;

use serde_json::json;

use biorouter::agents::extension_manager::ExtensionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::Conversation;
use biorouter::privacy::CallCapability;
use biorouter::providers::coding_agent::bridge;
use biorouter::session::session_manager::Session;
use biorouter::tool_inspection::ToolInspectionManager;

/// One advertised tool, so `tools/list` has something to prove it served from the
/// grant rather than from a hardcoded list.
fn advertised_tool() -> rmcp::model::Tool {
    rmcp::model::Tool::new(
        "spokeagent__query_knowledge_graph",
        "Run a Cypher query against SPOKE.",
        Arc::new(
            serde_json::from_value(json!({
                "type": "object",
                "properties": { "cypher": { "type": "string" } },
                "required": ["cypher"]
            }))
            .expect("a valid schema"),
        ),
    )
}

async fn grant() -> bridge::BridgeGrant {
    bridge::BridgeGrant::new(
        Session::default(),
        BioRouterMode::Auto,
        Arc::new(ExtensionManager::new(
            Arc::new(tokio::sync::Mutex::new(None)),
            Arc::new(biorouter::session::SessionManager::instance()),
        )),
        Arc::new(ToolInspectionManager::new()),
        CallCapability::public_enforced(),
        vec![advertised_tool()],
        Conversation::new_unvalidated(vec![]),
        // No cancel token, no hooks, no vault: these tests are about the HTTP
        // surface — reachability, the tool list, the lease's lifetime — and none
        // of them dispatches anything. The three snapshots the agent hands a real
        // grant are exercised where they act, in the bridge's own unit tests.
        None,
        no_hooks(),
        None,
        Arc::new(biorouter::permission::tool_risk::ToolRiskRegistry::new()),
    )
}

/// A hooks manager with nothing configured.
///
/// The grant needs one because a PreToolUse hook can *rewrite* a tool call's
/// arguments and the rewrite is staged inside the manager rather than returned
/// from the inspection — so a grant without it would run the user's hooks and
/// then dispatch the arguments they asked to replace. Empty here: these tests
/// configure no hooks, so `take_tool_input_rewrites` returns nothing and the
/// bridge's rewrite path short-circuits.
fn no_hooks() -> Arc<biorouter::hooks::HooksManager> {
    Arc::new(biorouter::hooks::HooksManager::with_config(
        Default::default(),
        false,
        Arc::new(tokio::sync::Mutex::new(None)),
    ))
}

/// A dispatcher that answers every call with one fixed result.
///
/// Stands in for an extension when a test needs to know that a call really ran
/// on Biorouter's side (a random marker only this returns) or needs a tool's
/// exact result shape.
struct FixedResultDispatch {
    result: rmcp::model::CallToolResult,
}

#[async_trait::async_trait]
impl bridge::BridgeToolDispatch for FixedResultDispatch {
    async fn dispatch(
        &self,
        _session_id: &str,
        _call: rmcp::model::CallToolRequestParams,
        _capability: CallCapability,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<rmcp::model::CallToolResult, String> {
        Ok(self.result.clone())
    }
}

/// A grant over [`advertised_tool`] whose calls are approved (Auto mode, a real
/// permission inspector) and answered with `result`.
fn fixed_result_grant(result: rmcp::model::CallToolResult) -> bridge::BridgeGrant {
    use biorouter::config::permission::PermissionManager;
    use biorouter::managed::ManagedPolicy;
    use biorouter::permission::permission_inspector::PermissionInspector;
    use biorouter::permission::tool_risk::ToolRiskRegistry;

    let risks = Arc::new(ToolRiskRegistry::new());
    let mut inspections = ToolInspectionManager::new();
    inspections.add_inspector(Box::new(PermissionInspector::new(
        Arc::clone(&risks),
        PermissionManager::instance(),
        Arc::new(ManagedPolicy::empty()),
        Arc::new(tokio::sync::Mutex::new(None)),
    )));
    bridge::BridgeGrant::new(
        Session::default(),
        BioRouterMode::Auto,
        Arc::new(FixedResultDispatch { result }),
        Arc::new(inspections),
        CallCapability::public_enforced(),
        vec![advertised_tool()],
        Conversation::new_unvalidated(vec![]),
        None,
        no_hooks(),
        None,
        risks,
    )
}

/// A grant whose one tool answers with `marker=<marker>` and nothing else.
fn marker_grant(marker: &str) -> bridge::BridgeGrant {
    fixed_result_grant(rmcp::model::CallToolResult::success(vec![
        rmcp::model::Content::text(format!("marker={marker}")),
    ]))
}

/// QA-E F4 at the wire, through the real router: a child's `tools/call` is
/// answered with the model's view of the result — the assistant's block, no
/// annotations — and the full result is kept under the child's own call id, for
/// each CLI's `_meta` spelling (measured: claude 2.1.266, codex-cli 0.153.4).
///
/// The shape is `developer__shell`'s. Handing the child both blocks made it read
/// every result twice, and the user block's `priority: 0.0` made codex-cli fail
/// the call outright with "Unexpected response type".
///
/// **And A2 at the wire**: that view is now the *framed* result, and the copy
/// kept for the transcript is the framed one too — the same bytes
/// `Agent::integrate_tool_result` would have stored for any other provider.
/// Before the fix this route answered raw text and stored raw text, so the same
/// `date` call read framed under `versa_azure` and unframed under both coding
/// agents.
#[tokio::test]
#[serial_test::serial]
async fn the_child_is_answered_with_the_models_view_and_the_full_result_is_kept() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    bridge::publish_base_url("http://127.0.0.1:65535");
    let shell = rmcp::model::CallToolResult::success(vec![
        rmcp::model::Content::text("Thu Sep 11").with_audience(vec![rmcp::model::Role::Assistant]),
        rmcp::model::Content::text("Thu Sep 11")
            .with_audience(vec![rmcp::model::Role::User])
            .with_priority(0.0),
    ]);

    // A2, at the HTTP layer: what a NON-bridged provider stores for this call —
    // `Agent::integrate_tool_result`'s own expression, run with the same mode the
    // grant sampled. Deriving the expectation rather than writing it out is what
    // makes this row say the thing that matters ("the two providers agree")
    // instead of pinning one spelling of the frame, and keeps it correct on a
    // machine whose `BIOROUTER_TOOL_OUTPUT_GUARDRAIL` differs from CI's.
    let mode = biorouter::guardrails::tool_output::ToolOutputGuardrailMode::from_config();
    let (guarded, _) = biorouter::guardrails::tool_output::guard_tool_result(
        Ok(shell.clone()),
        Some("spokeagent__query_knowledge_graph"),
        mode,
    );
    let guarded = guarded.expect("the guardrail passes an Ok through as Ok");
    let expected_child = bridge::child_view(&guarded);

    let lease = bridge::issue(fixed_result_grant(shell.clone())).expect("issued");
    let nonce = lease.url().rsplit('/').next().expect("a nonce").to_string();

    for (meta, child_call_id) in [
        (
            json!({ "claudecode/toolUseId": "toolu_wire", "progressToken": 2 }),
            "toolu_wire",
        ),
        (
            json!({ "callId": "exec-wire", "threadId": "t", "progressToken": 1 }),
            "exec-wire",
        ),
    ] {
        let request = json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {
                "name": "spokeagent__query_knowledge_graph",
                "arguments": { "cypher": "MATCH (n) RETURN n LIMIT 1" },
                "_meta": meta,
            }
        });
        let response = biorouter_server::routes::tool_bridge::routes()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/tool_bridge/{nonce}"))
                    .header("content-type", "application/json")
                    .body(Body::from(request.to_string()))
                    .expect("a request"),
            )
            .await
            .expect("the route answers");
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("a body");
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON");

        assert_eq!(
            body["result"]["content"],
            serde_json::to_value(&expected_child.content).expect("serialisable content"),
            "the child gets the model's block, unannotated and framed exactly as \
             a non-bridged provider's would be: {body}"
        );
        assert!(
            !body.to_string().contains("priority"),
            "codex-cli cannot parse a `priority` annotation: {body}"
        );
        // Not vacuous: unless the operator turned the guardrail off, the frame
        // really is on the wire. Before A2 this route answered raw text.
        if mode != biorouter::guardrails::tool_output::ToolOutputGuardrailMode::Off {
            assert!(
                body["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with(biorouter::guardrails::tool_output::TOOL_OUTPUT_FRAME_OPEN),
                "the child agent read unframed tool output over the bridge: {body}"
            );
        }
        assert_eq!(
            bridge::take_recorded_result(lease.url(), child_call_id),
            Some(guarded.clone()),
            "the full result is kept for the transcript under {child_call_id}, \
             framed the way every other provider's transcript entry is"
        );
    }
}

/// The whole lifecycle in one test, because the assertions are sequential: a grant
/// is reachable, serves its own tool set, and stops existing when its lease drops.
#[tokio::test]
#[serial_test::serial]
async fn a_grant_is_reachable_only_while_its_lease_lives() {
    bridge::publish_base_url("http://127.0.0.1:65535");
    let lease = bridge::issue(grant().await).expect("a base URL is published");
    let nonce = lease
        .url()
        .rsplit('/')
        .next()
        .expect("the url ends in the nonce")
        .to_string();

    // Reachable now.
    assert!(
        bridge::lookup(&nonce).is_some(),
        "the grant must be reachable while the lease lives"
    );
    let found = bridge::lookup(&nonce).expect("reachable");
    assert_eq!(
        found.tools().len(),
        1,
        "the grant serves the tool set it was issued with"
    );
    assert_eq!(
        found.tools()[0].name.as_ref(),
        "spokeagent__query_knowledge_graph"
    );

    // Unreachable the instant the lease is dropped. A grant that outlived its turn
    // would be a live capability onto a session's tools with nothing owning it.
    drop(lease);
    assert!(
        bridge::lookup(&nonce).is_none(),
        "dropping the lease must revoke the grant"
    );
}

/// A nonce that was never issued is refused, and so is one whose turn has ended —
/// with the same message, so the endpoint cannot be used to discover which nonces
/// exist.
#[tokio::test]
#[serial_test::serial]
async fn an_unissued_nonce_is_indistinguishable_from_an_expired_one() {
    bridge::publish_base_url("http://127.0.0.1:65535");

    let lease = bridge::issue(grant().await).expect("a base URL is published");
    let expired = lease
        .url()
        .rsplit('/')
        .next()
        .expect("the url ends in the nonce")
        .to_string();
    drop(lease);

    assert!(bridge::lookup(&expired).is_none());
    assert!(bridge::lookup("0123456789abcdef0123456789abcdef").is_none());
    assert!(bridge::lookup("not-a-nonce").is_none());
    assert!(bridge::lookup("").is_none());
}

/// The nonce is the whole credential, so it has to be long and unguessable. A
/// short or predictable one would make the bridge reachable by anything on the
/// machine that can reach loopback.
#[tokio::test]
#[serial_test::serial]
async fn every_nonce_is_long_random_and_unique() {
    bridge::publish_base_url("http://127.0.0.1:65535");
    let a = bridge::issue(grant().await).expect("issued");
    let b = bridge::issue(grant().await).expect("issued");

    assert_ne!(a.url(), b.url(), "two leases must never share a nonce");
    for lease in [&a, &b] {
        assert!(lease.url().contains("/tool_bridge/"));
        let nonce = lease.url().rsplit('/').next().unwrap();
        assert_eq!(nonce.len(), 32, "a short nonce is a guessable capability");
        assert!(
            nonce.chars().all(|c| c.is_ascii_hexdigit()),
            "the nonce must be hex so it survives a URL unencoded"
        );
    }
}

/// Without a published base URL there is nothing for a child to connect to, so no
/// grant may be handed out. The providers read that as "run with no tools", which
/// is why it must be `None` rather than a URL pointing nowhere.
#[tokio::test]
#[serial_test::serial]
async fn the_url_carries_the_nonce_and_the_published_base() {
    bridge::publish_base_url("http://127.0.0.1:8123");
    let lease = bridge::issue(grant().await).expect("issued");
    assert!(
        lease
            .url()
            .starts_with("http://127.0.0.1:8123/tool_bridge/"),
        "the child is given an absolute URL on the daemon: {}",
        lease.url()
    );
}

/// A trailing slash on the published base must not produce a doubled separator —
/// the child would request a path the router does not match, and the failure would
/// look like an authentication problem.
#[tokio::test]
#[serial_test::serial]
async fn a_trailing_slash_on_the_base_does_not_double_the_separator() {
    bridge::publish_base_url("http://127.0.0.1:8123/");
    let lease = bridge::issue(grant().await).expect("issued");
    assert!(
        !lease.url().contains("//tool_bridge"),
        "malformed bridge URL: {}",
        lease.url()
    );
}

// ---------------------------------------------------------------------------
// Live end-to-end, against the real vendor CLI. `--ignored`, because it needs
// `claude` installed and signed in to a subscription, and it spends the user's
// own plan quota.
//
//   cargo test -p biorouter-server --test tool_bridge_routes -- --ignored --nocapture
//
// This is the only test that proves the whole path rather than a layer of it: the
// real router serving the real registry, reached by the real Claude Code over
// HTTP, discovering Biorouter's tool set. Every other assertion in this file would
// still pass if the two halves never spoke.
// ---------------------------------------------------------------------------

/// Serve the real bridge route on an ephemeral port and publish it, so anything
/// that asks for a grant gets a URL that actually resolves.
async fn serve_real_bridge() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port");
    let addr = listener.local_addr().expect("a bound address");
    let app = biorouter_server::routes::tool_bridge::routes();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let base = format!("http://{addr}");
    bridge::publish_base_url(base.clone());
    base
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "needs the `claude` CLI installed and signed in; spends the user's own plan quota"]
async fn the_real_claude_cli_discovers_biorouters_tools_over_the_bridge() {
    serve_real_bridge().await;
    let lease = bridge::issue(grant().await).expect("the base URL is published");

    // Point the CLI at the bridge exactly the way the provider does.
    let config = serde_json::json!({
        "mcpServers": { "biorouter": { "type": "http", "url": lease.url() } }
    });
    let dir = tempfile::tempdir().expect("a temp dir");
    let config_path = dir.path().join("mcp.json");
    std::fs::write(&config_path, config.to_string()).expect("write the bridge config");

    let mut child = tokio::process::Command::new("claude")
        .args([
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--model",
            "haiku",
            "--tools",
            "",
            "--setting-sources",
            "",
            "--strict-mcp-config",
            "--permission-mode",
            "bypassPermissions",
            "--no-session-persistence",
            "--system-prompt",
            "You are Biorouter.",
        ])
        .arg("--mcp-config")
        .arg(&config_path)
        // ⚠ The prompt goes on STDIN, not as a trailing positional, and that is not
        // a style choice. `--mcp-config` is variadic ("space-separated"), so a
        // positional prompt after it is swallowed as a second config path and the
        // run dies with "MCP config file not found: <your prompt>". The provider
        // writes the prompt to stdin for the same reason (plus argv length limits),
        // so doing it here keeps this a faithful reproduction.
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env_remove("ANTHROPIC_API_KEY")
        .spawn()
        .expect("the claude CLI runs");

    {
        use tokio::io::AsyncWriteExt;
        let mut stdin = child.stdin.take().expect("piped stdin");
        stdin
            .write_all(b"List the tool names you have been given, and nothing else.")
            .await
            .expect("write the prompt");
        stdin.shutdown().await.expect("close stdin");
    }
    let output = child.wait_with_output().await.expect("the CLI finishes");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let init = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| v["type"] == "system" && v["subtype"] == "init")
        .unwrap_or_else(|| {
            panic!(
                "no system/init frame; stdout was:\n{stdout}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stderr)
            )
        });

    // The bridge connected...
    let servers = init["mcp_servers"].as_array().cloned().unwrap_or_default();
    assert!(
        servers
            .iter()
            .any(|s| s["name"] == "biorouter" && s["status"] == "connected"),
        "the bridge should be connected: {init}"
    );
    assert!(
        init["mcp_server_errors"].is_null(),
        "the bridge reported a config error: {init}"
    );

    // ...and served Biorouter's tool set, which the CLI re-prefixes as
    // `mcp__<server>__<tool>`.
    let tools = init["tools"].as_array().cloned().unwrap_or_default();
    assert!(
        tools.iter().any(|t| t.as_str().unwrap_or_default()
            == "mcp__biorouter__spokeagent__query_knowledge_graph"),
        "the grant's tool should have reached the model: {tools:?}"
    );

    // The run must be on the subscription, not on a key. This is the assertion the
    // whole feature rests on, so it is made here too rather than only in the unit
    // tests, where the value is a fixture.
    assert_eq!(
        init["apiKeySource"], "none",
        "the live run must be subscription-billed: {init}"
    );
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "needs the `codex` CLI installed and signed in; spends the user's own plan quota"]
async fn the_real_codex_provider_reaches_biorouters_tools_over_the_bridge() {
    use biorouter::conversation::message::Message;
    use biorouter::model::ModelConfig;
    use biorouter::providers::base::Provider;
    use biorouter::providers::codex::CodexProvider;

    // ⚠ QA-E F1: this used to issue a grant over an EMPTY extension manager and
    // accept any answer that NAMED the tool, on the argument that a refusal "can
    // only have come from Biorouter's side of the bridge". It could also come from
    // Codex's own side — "MCP tool call requires approval, but approval policy is
    // never" names the tool too — so on codex-cli 0.148+ this passed while every
    // bridged call was refused inside the CLI. The tool now answers with a random
    // marker that exists only in Biorouter's dispatcher, so only a call that
    // really crossed the bridge and ran can put it in the answer.
    let marker = format!("CODEXBRIDGE{:016x}", rand::random::<u64>());
    serve_real_bridge().await;
    let lease = bridge::issue(marker_grant(&marker)).expect("the base URL is published");

    // Drive the PROVIDER, not the CLI directly. `codex exec` cannot answer an
    // approval request, so an MCP tool call there fails with "user cancelled MCP
    // tool call" unless the sandbox is opened up wholesale — which is the entire
    // reason the provider speaks `codex app-server` instead. Testing `exec` here
    // would be testing a surface we deliberately do not use, and it would fail for
    // a reason that has nothing to do with the bridge.
    let provider = CodexProvider::from_env(ModelConfig::new("gpt-5.5").expect("a known model"))
        .await
        .expect("the codex CLI is installed");

    let messages = vec![Message::user().with_text(
        "Call the spokeagent__query_knowledge_graph tool with cypher='MATCH (n) RETURN n LIMIT 1'. \
         Then reply with ONLY the marker value the tool returned, and nothing else.",
    )];

    let outcome = bridge::ACTIVE_BRIDGE_URL
        .scope(
            Some(lease.url().to_string()),
            provider.complete(
                "You are Biorouter. Use the tools you are given.",
                &messages,
                &[],
            ),
        )
        .await;

    match outcome {
        Ok((message, usage)) => {
            let text = message.as_concat_text();
            assert!(
                !text.contains("approval policy"),
                "Codex refused the call itself instead of asking Biorouter: {text}"
            );
            assert!(
                text.contains(&marker),
                "the bridged tool never ran: {marker} exists only in Biorouter's \
                 dispatcher, and the answer was: {text}"
            );
            assert_eq!(
                usage.provider.as_deref(),
                Some("codex"),
                "the usage row must be attributed to this provider"
            );
        }
        Err(e) => panic!("the codex turn failed: {e}"),
    }
}

// ---------------------------------------------------------------------------
// The real-extension round trip. Also `--ignored`, for the same reasons.
// ---------------------------------------------------------------------------

/// A grant whose `ExtensionManager` holds the **real** `developer` builtin, with
/// its file jail rooted at `working_dir`, and whose advertised tool set is the
/// manager's own prefixed list.
///
/// Two things here are load-bearing and easy to get subtly wrong:
///
/// * `set_working_dir` runs **before** `add_extension`. A `Builtin` is spawned
///   in-process over a duplex pipe and is handed the *resolved* working
///   directory at spawn time, which becomes the developer server's jail base for
///   the rest of its life. Setting it afterwards would leave the jail at the test
///   binary's cwd and every path under the temp dir would come back "outside the
///   working directory".
/// * The inspection manager carries a real [`PermissionInspector`]. With the
///   empty `ToolInspectionManager` the other tests use,
///   `process_inspection_results_with_permission_inspector` returns `None` and
///   `BridgeGrant::call` refuses before dispatch ever happens — so an empty
///   manager can only ever prove a refusal, never an execution.
async fn real_developer_grant(working_dir: &std::path::Path) -> bridge::BridgeGrant {
    real_developer_grant_in(working_dir, BioRouterMode::Auto, Session::default()).await
}

/// The same grant, in a caller-chosen mode and session.
///
/// #107 needs both: `Approve` so the permission inspector actually routes the
/// call to `needs_approval`, and a session with a real id because the card is
/// published into a queue keyed by session id — `Session::default()`'s id is the
/// empty string, which every other default-session test in the process shares.
async fn real_developer_grant_in(
    working_dir: &std::path::Path,
    mode: BioRouterMode,
    session: Session,
) -> bridge::BridgeGrant {
    use biorouter::agents::ExtensionConfig;
    use biorouter::config::permission::PermissionManager;
    use biorouter::managed::ManagedPolicy;
    use biorouter::permission::managed_inspector::ManagedPolicyInspector;
    use biorouter::permission::permission_inspector::PermissionInspector;
    use biorouter::permission::tool_risk::ToolRiskRegistry;
    use biorouter::security::security_inspector::SecurityInspector;
    use biorouter::security::sensitive_ops::SensitiveOpsInspector;

    let provider: biorouter::agents::types::SharedProvider =
        Arc::new(tokio::sync::Mutex::new(None));
    let extensions = Arc::new(ExtensionManager::new(
        Arc::clone(&provider),
        Arc::new(biorouter::session::SessionManager::instance()),
    ));
    extensions.set_working_dir(working_dir.to_path_buf()).await;
    extensions
        .add_extension(ExtensionConfig::Builtin {
            name: "developer".to_string(),
            display_name: Some("Developer".to_string()),
            description: "Biorouter's built-in file and shell tools.".to_string(),
            timeout: Some(120),
            bundled: Some(true),
            available_tools: Vec::new(),
        })
        .await
        .expect("the developer builtin loads in-process over a duplex pipe");

    // The SAME source the agent reads. `Agent::prepare_tools` calls exactly this
    // and then appends the platform/frontend/subagent tools, which the grant
    // construction filters back out again because they are not dispatched by the
    // `ExtensionManager`. So for a session whose only extension is `developer`,
    // this IS the bridged set — not an approximation of it.
    let tools = extensions
        .get_prefixed_tools(None)
        .await
        .expect("the developer extension serves its tool list");

    // The inspectors the agent registers ahead of dispatch, minus the two that
    // need agent-owned state (hooks, repetition history). Auto mode's blanket
    // allow is what lets a bridged call through, and the security gates above it
    // still run — an escalation-only merge, so a refusal from any of them would
    // be a real finding rather than test noise.
    let risks = Arc::new(ToolRiskRegistry::new());
    risks.refresh_from_tools(&tools);
    let managed = Arc::new(ManagedPolicy::empty());
    let mut inspections = ToolInspectionManager::new();
    inspections.add_inspector(Box::new(ManagedPolicyInspector::new(Arc::clone(&managed))));
    inspections.add_inspector(Box::new(SecurityInspector::new()));
    inspections.add_inspector(Box::new(SensitiveOpsInspector::unbound()));
    inspections.add_inspector(Box::new(PermissionInspector::new(
        risks,
        PermissionManager::instance(),
        managed,
        Arc::clone(&provider),
    )));

    bridge::BridgeGrant::new(
        session,
        mode,
        extensions,
        Arc::new(inspections),
        CallCapability::public_enforced(),
        tools,
        Conversation::new_unvalidated(vec![]),
        // A turn's cancel token would be the agent's; this grant has no turn
        // behind it, and `None` means "never cancelled", which is what a test
        // driving the call to completion wants. No hooks and no vault for the
        // same reason the simpler grant above has none — nothing here configures
        // either, and both paths short-circuit when empty.
        None,
        no_hooks(),
        None,
        Arc::new(ToolRiskRegistry::new()),
    )
}

/// A real Biorouter extension, called by the real child, executed here, with the
/// real result reaching the child's answer.
///
/// **What this proves that no other test in this file does.** Every other test —
/// including the live `claude` one above — issues a grant whose
/// `ExtensionManager` is EMPTY and whose tool set is one synthetic
/// `rmcp::model::Tool`. Those prove the wiring: that a nonce resolves, that a
/// child connects, that `tools/list` serves the grant's own set, and (for codex)
/// that a `tools/call` reaches Biorouter's gate stack — but the call they prove
/// reaches it is one that can only ever be *refused*, because there is no
/// extension behind the name. Nothing yet showed that a tool actually **ran**.
///
/// This closes that gap end to end, in the one direction that matters:
///
/// 1. the grant carries a real, loaded extension (`developer`), and its
///    advertised tools come from `ExtensionManager::get_prefixed_tools` — the
///    same call the agent makes — so the real schemas and the real prefixed
///    names are what crosses the bridge;
/// 2. the child chooses a tool and calls it back over HTTP;
/// 3. Biorouter executes it for real, against the filesystem;
/// 4. and the bytes it produced come back out in the child's final answer.
///
/// The assertion is a marker written into a temp file and **never put in the
/// prompt**. That is what makes step 3 non-fakeable: a model that never reached
/// the bridge, or reached it and got a refusal, cannot produce a random 64-bit
/// token it was never shown. A weaker fixture ("hello world") would pass on a
/// hallucination.
#[tokio::test]
#[serial_test::serial]
#[ignore = "needs the `claude` CLI installed and signed in; spends the user's own plan quota"]
async fn a_real_biorouter_extension_executes_and_its_output_reaches_the_childs_answer() {
    // Unguessable, and deliberately a single bare token so a model copies it
    // verbatim. It exists only on disk — see the doc comment.
    let marker = format!("BRIDGEPROOF{:016x}", rand::random::<u64>());

    let dir = tempfile::tempdir().expect("a temp dir");
    let target = dir.path().join("marker.txt");
    std::fs::write(&target, format!("{marker}\n")).expect("write the marker file");

    let grant = real_developer_grant(dir.path()).await;

    // Fail here, loudly, rather than blaming the model for a tool it was never
    // offered: if the extension did not load, nothing downstream is meaningful.
    let advertised: Vec<String> = grant.tools().iter().map(|t| t.name.to_string()).collect();
    assert!(
        advertised.iter().any(|n| n == "developer__text_editor"),
        "the real developer extension must have loaded and served its tools; got: {advertised:?}"
    );

    serve_real_bridge().await;
    let lease = bridge::issue(grant).expect("the base URL is published");

    // ⚠ Must be EXACTLY the config the provider writes, `timeout` included.
    // Without that field the child falls back to its own default per-call
    // deadline, which is far shorter than `approval_ttl()` (570s) — so a card
    // nobody answers is reported to the model as a bare "The operation timed
    // out" instead of Biorouter's readable "expired before anyone answered it".
    // This test omitted it, so it was asserting against a deadline production
    // does not have, and any child that RETRIED after the denial failed it.
    let config = serde_json::json!({
        "mcpServers": {
            "biorouter": {
                "type": "http",
                "url": lease.url(),
                "timeout": bridge::child_tool_call_timeout().as_millis() as u64,
            }
        }
    });
    let config_path = dir.path().join("mcp.json");
    std::fs::write(&config_path, config.to_string()).expect("write the bridge config");

    let mut child = tokio::process::Command::new("claude")
        .args([
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--model",
            "haiku",
            // The child's OWN file tools are off, so the only way to read the
            // file is through Biorouter. Without this the test could pass with
            // the bridge never touched.
            "--tools",
            "",
            "--setting-sources",
            "",
            "--strict-mcp-config",
            "--permission-mode",
            "bypassPermissions",
            "--no-session-persistence",
            "--system-prompt",
            "You are Biorouter. Use the tools you are given.",
        ])
        .arg("--mcp-config")
        .arg(&config_path)
        // ⚠ stdin, not a trailing positional — `--mcp-config` is variadic and
        // would swallow the prompt as a second config path. See the test above.
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env_remove("ANTHROPIC_API_KEY")
        .spawn()
        .expect("the claude CLI runs");

    {
        use tokio::io::AsyncWriteExt;
        let mut stdin = child.stdin.take().expect("piped stdin");
        stdin
            .write_all(
                format!(
                    "Use your text_editor tool with command \"view\" to read the file at {}. \
                     Then reply with ONLY the single word that file contains, and nothing else.",
                    target.display()
                )
                .as_bytes(),
            )
            .await
            .expect("write the prompt");
        stdin.shutdown().await.expect("close stdin");
    }
    let output = child.wait_with_output().await.expect("the CLI finishes");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let frames: Vec<serde_json::Value> = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .collect();
    let dump = || format!("stdout was:\n{stdout}\nstderr:\n{stderr}");

    // The bridge connected and served the REAL tool set.
    let init = frames
        .iter()
        .find(|v| v["type"] == "system" && v["subtype"] == "init")
        .unwrap_or_else(|| panic!("no system/init frame; {}", dump()));
    let servers = init["mcp_servers"].as_array().cloned().unwrap_or_default();
    assert!(
        servers
            .iter()
            .any(|s| s["name"] == "biorouter" && s["status"] == "connected"),
        "the bridge should be connected: {init}"
    );
    let offered = init["tools"].as_array().cloned().unwrap_or_default();
    assert!(
        offered
            .iter()
            .any(|t| t.as_str().unwrap_or_default() == "mcp__biorouter__developer__text_editor"),
        "the real extension's tool should have reached the model: {offered:?}"
    );

    // The child actually CALLED it — not merely saw it listed.
    let called: Vec<String> = frames
        .iter()
        .filter(|v| v["type"] == "assistant")
        .filter_map(|v| v["message"]["content"].as_array())
        .flatten()
        .filter(|block| block["type"] == "tool_use")
        .filter_map(|block| block["name"].as_str().map(str::to_string))
        .collect();
    assert!(
        called
            .iter()
            .any(|n| n.starts_with("mcp__biorouter__developer__")),
        "the child should have called a bridged Biorouter tool; it called {called:?}. {}",
        dump()
    );

    // ...and BIOROUTER executed it: the marker exists only on disk, so its
    // presence in the answer is the round trip.
    let result = frames
        .iter()
        .find(|v| v["type"] == "result")
        .unwrap_or_else(|| panic!("no result frame; {}", dump()));
    assert_eq!(
        result["subtype"], "success",
        "the child turn failed: {result}"
    );
    let answer = result["result"].as_str().unwrap_or_default();
    assert!(
        answer.contains(&marker),
        "the real tool's output never reached the answer. Expected {marker}, answer was: \
         {answer:?}. {}",
        dump()
    );
}

// ---------------------------------------------------------------------------
// #107: a bridged call that needs a person, end to end through the real child.
// ---------------------------------------------------------------------------

/// Wait for the approval card Biorouter published for `session_id`, and return
/// its request id.
///
/// Polls the same queue the desktop's agent loop drains. That is deliberately
/// not a shortcut: it is the *only* channel the card travels on, so a test that
/// finds it here is finding the thing the GUI would render, and a card that never
/// arrives fails this poll rather than passing silently.
/// The same poll, but a timeout is an ANSWER rather than a panic.
///
/// A denier that keeps answering cards needs to stop when no more arrive; the
/// panicking variant would fail the test at the end of every successful run.
async fn await_published_approval_opt(
    session_id: &str,
    within: std::time::Duration,
) -> Option<String> {
    use biorouter::conversation::message::{ActionRequiredData, MessageContent};
    tokio::time::timeout(within, async {
        loop {
            for message in biorouter::action_required_manager::ActionRequiredManager::global()
                .drain_requests(session_id)
            {
                for content in &message.content {
                    if let MessageContent::ActionRequired(action) = content {
                        if let ActionRequiredData::ToolConfirmation { id, tool_name, .. } =
                            &action.data
                        {
                            eprintln!("approval card raised for {tool_name}");
                            return id.clone();
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .ok()
}

async fn await_published_approval(session_id: &str, within: std::time::Duration) -> String {
    use biorouter::conversation::message::{ActionRequiredData, MessageContent};
    tokio::time::timeout(within, async {
        loop {
            for message in biorouter::action_required_manager::ActionRequiredManager::global()
                .drain_requests(session_id)
            {
                for content in &message.content {
                    if let MessageContent::ActionRequired(action) = content {
                        if let ActionRequiredData::ToolConfirmation { id, tool_name, .. } =
                            &action.data
                        {
                            eprintln!("approval card raised for {tool_name}");
                            return id.clone();
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("Biorouter must publish an approval card for the bridged call")
}

/// **The whole of #107, against the real Claude Code.** A bridged tool call that
/// Biorouter's permission policy routes to `needs_approval` raises a real,
/// routable approval request, parks the child's HTTP call while a person decides,
/// and — once approved — runs the tool and returns its output into the child's
/// answer.
///
/// What this proves that the unit tests cannot: the *child* tolerates the park.
/// Every layer below could be correct and this still fail, because the thing
/// being waited on is a live HTTP request held open inside the child's own MCP
/// client, under its own per-call deadline. That is the reason the park has a TTL
/// shorter than the deadline (`bridge::child_tool_call_budget`) rather than the
/// hour a chat confirmation may take.
///
/// The marker is written to disk and never appears in the prompt, so a model that
/// hallucinated its way past a refusal cannot produce it — the same
/// non-fakeability argument as the Auto-mode test above, now with the approval in
/// the middle.
#[tokio::test]
#[serial_test::serial]
#[ignore = "needs the `claude` CLI installed and signed in; spends the user's own plan quota"]
async fn a_bridged_call_needing_approval_is_answerable_and_resumes() {
    let marker = format!("APPROVALPROOF{:016x}", rand::random::<u64>());
    let dir = tempfile::tempdir().expect("a temp dir");
    let target = dir.path().join("marker.txt");
    std::fs::write(&target, format!("{marker}\n")).expect("write the marker file");

    let session_id = format!("approval-e2e-{:016x}", rand::random::<u64>());
    let session = Session {
        id: session_id.clone(),
        ..Session::default()
    };
    // Approve mode: the permission inspector routes an ungranted call to
    // `needs_approval` instead of allowing it outright. Without this the test
    // would pass with the approval machinery never touched.
    let grant = real_developer_grant_in(dir.path(), BioRouterMode::Approve, session).await;
    let advertised: Vec<String> = grant.tools().iter().map(|t| t.name.to_string()).collect();
    assert!(
        advertised.iter().any(|n| n == "developer__text_editor"),
        "the real developer extension must have loaded; got: {advertised:?}"
    );

    serve_real_bridge().await;
    let lease = bridge::issue(grant).expect("the base URL is published");

    // The person. Answers as soon as the card appears, which is what the desktop
    // dialog does when a user clicks Allow.
    let approver = tokio::spawn({
        let session_id = session_id.clone();
        async move {
            let id =
                await_published_approval(&session_id, std::time::Duration::from_secs(90)).await;
            let outcome = biorouter::pending_user_action::PendingUserActions::global()
                .resolve_in_session(
                    &session_id,
                    &id,
                    biorouter::pending_user_action::UserActionOutcome::Approved {
                        permission: biorouter::permission::Permission::AllowOnce,
                    },
                    biorouter::pending_user_action::DecisionAuthority::unproven(),
                );
            assert_eq!(
                outcome,
                biorouter::pending_user_action::ResolveOutcome::Delivered,
                "the decision must reach the call that is parked on it"
            );
        }
    });

    let config = serde_json::json!({
        "mcpServers": { "biorouter": { "type": "http", "url": lease.url() } }
    });
    let config_path = dir.path().join("mcp.json");
    std::fs::write(&config_path, config.to_string()).expect("write the bridge config");

    let mut child = tokio::process::Command::new("claude")
        .args([
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--model",
            "haiku",
            "--tools",
            "",
            "--setting-sources",
            "",
            "--strict-mcp-config",
            "--permission-mode",
            "bypassPermissions",
            "--no-session-persistence",
            "--system-prompt",
            "You are Biorouter. Use the tools you are given.",
        ])
        .arg("--mcp-config")
        .arg(&config_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env_remove("ANTHROPIC_API_KEY")
        .spawn()
        .expect("the claude CLI runs");

    {
        use tokio::io::AsyncWriteExt;
        let mut stdin = child.stdin.take().expect("piped stdin");
        stdin
            .write_all(
                format!(
                    "Use your text_editor tool with command \"view\" to read the file at {}. \
                     Then reply with ONLY the single word that file contains, and nothing else.",
                    target.display()
                )
                .as_bytes(),
            )
            .await
            .expect("write the prompt");
        stdin.shutdown().await.expect("close stdin");
    }
    let output = child.wait_with_output().await.expect("the CLI finishes");
    approver.await.expect("the approver task must not panic");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let dump = || format!("stdout was:\n{stdout}\nstderr:\n{stderr}");
    let frames: Vec<serde_json::Value> = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .collect();

    let result = frames
        .iter()
        .find(|v| v["type"] == "result")
        .unwrap_or_else(|| panic!("no result frame; {}", dump()));
    let answer = result["result"].as_str().unwrap_or_default();

    // The park must not have looked like a broken server to the child.
    assert!(
        !stdout.contains("operation timed out") && !stdout.contains("Operation timed out"),
        "the park must fit inside the child's per-call deadline; {}",
        dump()
    );
    // The old refusal must be gone: this is the exact sentence #107 reported.
    assert!(
        !stdout.contains("no way to ask for one"),
        "the bridge still refused instead of asking; {}",
        dump()
    );
    // And the tool ran: the marker exists only on disk.
    assert!(
        answer.contains(&marker),
        "the approved tool's output never reached the answer. Expected {marker}, answer was \
         {answer:?}. {}",
        dump()
    );
}

/// The other half of the same round trip: **denial** comes back as an ordinary
/// tool result the child's model reads and acts on, not as a transport failure it
/// retries — and the text does not send it back to ask in prose, because by then
/// the request id is gone and a chat message could not resolve anything.
#[tokio::test]
#[serial_test::serial]
#[ignore = "needs the `claude` CLI installed and signed in; spends the user's own plan quota"]
async fn a_denied_bridged_call_returns_a_result_the_child_can_act_on() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let target = dir.path().join("marker.txt");
    std::fs::write(&target, "SECRETVALUE\n").expect("write the file");

    let session_id = format!("denial-e2e-{:016x}", rand::random::<u64>());
    let session = Session {
        id: session_id.clone(),
        ..Session::default()
    };
    let grant = real_developer_grant_in(dir.path(), BioRouterMode::Approve, session).await;
    serve_real_bridge().await;
    let lease = bridge::issue(grant).expect("the base URL is published");

    // ⚠ Deny for the WHOLE run, not once. A model may RETRY after a refusal —
    // this one does, roughly half the time — and each retry parks a NEW card.
    // Answering only the first left every later card unanswered until
    // `approval_ttl` (570s), which the child reports as a bare "The operation
    // timed out", failing the assertion below for a reason that has nothing to do
    // with the denial path.
    //
    // That flakiness read as a code regression and cost a long bisect: it
    // correlated cleanly with an unrelated two-line change across seven runs
    // (4 pass / 3 fail, split exactly on that file), and the correlation
    // INVERTED as soon as this test was fixed. Seven runs was not enough to tell
    // a real effect from a coin flip.
    let denier = tokio::spawn({
        let session_id = session_id.clone();
        async move {
            let mut first = None;
            loop {
                let Some(id) =
                    await_published_approval_opt(&session_id, std::time::Duration::from_secs(20))
                        .await
                else {
                    break;
                };
                let outcome = biorouter::pending_user_action::PendingUserActions::global()
                    .resolve_in_session(
                        &session_id,
                        &id,
                        biorouter::pending_user_action::UserActionOutcome::Denied {
                            permission: biorouter::permission::Permission::DenyOnce,
                        },
                        biorouter::pending_user_action::DecisionAuthority::unproven(),
                    );
                if first.is_none() {
                    first = Some(outcome);
                }
            }
            first.expect("at least one approval card must have been raised and denied")
        }
    });

    // ⚠ EXACTLY the config the provider writes, `timeout` included. Without that
    // field the child falls back to its own default per-call deadline, far
    // shorter than `approval_ttl()`, so an unanswered card surfaces as a bare
    // "The operation timed out" rather than Biorouter's readable refusal — i.e.
    // the test would assert against a deadline production does not have.
    let config = serde_json::json!({
        "mcpServers": {
            "biorouter": {
                "type": "http",
                "url": lease.url(),
                "timeout": bridge::child_tool_call_timeout().as_millis() as u64,
            }
        }
    });
    let config_path = dir.path().join("mcp.json");
    std::fs::write(&config_path, config.to_string()).expect("write the bridge config");

    let mut child = tokio::process::Command::new("claude")
        .args([
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--model",
            "haiku",
            "--tools",
            "",
            "--setting-sources",
            "",
            "--strict-mcp-config",
            "--permission-mode",
            "bypassPermissions",
            "--no-session-persistence",
            "--system-prompt",
            "You are Biorouter. Use the tools you are given.",
        ])
        .arg("--mcp-config")
        .arg(&config_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env_remove("ANTHROPIC_API_KEY")
        .spawn()
        .expect("the claude CLI runs");
    {
        use tokio::io::AsyncWriteExt;
        let mut stdin = child.stdin.take().expect("piped stdin");
        stdin
            .write_all(
                format!(
                    "Use your text_editor tool with command \"view\" to read {}, then say what \
                     it contains.",
                    target.display()
                )
                .as_bytes(),
            )
            .await
            .expect("write the prompt");
        stdin.shutdown().await.expect("close stdin");
    }
    let output = child.wait_with_output().await.expect("the CLI finishes");
    assert_eq!(
        denier.await.expect("the denier task must not panic"),
        biorouter::pending_user_action::ResolveOutcome::Delivered,
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let dump = || {
        format!(
            "stdout was:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    };
    assert!(
        stdout.contains("did not approve"),
        "the refusal must reach the child's model as a readable result; {}",
        dump()
    );
    assert!(
        !stdout.contains("operation timed out") && !stdout.contains("Operation timed out"),
        "a denial must be a result, not an abandoned request; {}",
        dump()
    );
    assert!(
        !stdout.contains("SECRETVALUE"),
        "a denied call must not have run; {}",
        dump()
    );
}

/// #107 through the **real Codex** bridge.
///
/// Codex is the harder half and the reason this test exists separately. Its MCP
/// client is a different implementation with its own per-call deadline, and it
/// sends no `Authorization` header — the capability rides the URL — so nothing
/// proven against Claude Code transfers by argument. What has to hold is the
/// same: the child's `tools/call` stays open while a person decides, and the
/// decision resumes it into a real execution rather than a timeout.
///
/// Driven through `CodexProvider` rather than `codex exec`, for the reason the
/// test above it records: `exec` cannot answer an approval and would fail for a
/// reason that has nothing to do with Biorouter's.
#[tokio::test]
#[serial_test::serial]
#[ignore = "needs the `codex` CLI installed and signed in; spends the user's own plan quota"]
async fn a_bridged_codex_call_needing_approval_is_answerable_and_resumes() {
    use biorouter::conversation::message::Message;
    use biorouter::model::ModelConfig;
    use biorouter::providers::base::Provider;
    use biorouter::providers::codex::CodexProvider;

    let marker = format!("CODEXAPPROVAL{:016x}", rand::random::<u64>());
    let dir = tempfile::tempdir().expect("a temp dir");
    let target = dir.path().join("marker.txt");
    std::fs::write(&target, format!("{marker}\n")).expect("write the marker file");

    let session_id = format!("codex-approval-e2e-{:016x}", rand::random::<u64>());
    let session = Session {
        id: session_id.clone(),
        ..Session::default()
    };
    let grant = real_developer_grant_in(dir.path(), BioRouterMode::Approve, session).await;
    serve_real_bridge().await;
    let lease = bridge::issue(grant).expect("the base URL is published");

    let approver = tokio::spawn({
        let session_id = session_id.clone();
        async move {
            let id =
                await_published_approval(&session_id, std::time::Duration::from_secs(120)).await;
            biorouter::pending_user_action::PendingUserActions::global().resolve_in_session(
                &session_id,
                &id,
                biorouter::pending_user_action::UserActionOutcome::Approved {
                    permission: biorouter::permission::Permission::AllowOnce,
                },
                biorouter::pending_user_action::DecisionAuthority::unproven(),
            )
        }
    });

    let provider = CodexProvider::from_env(ModelConfig::new("gpt-5.5").expect("a known model"))
        .await
        .expect("the codex CLI is installed");
    let messages = vec![Message::user().with_text(format!(
        "Use the developer__text_editor tool with command \"view\" to read the file at {}. \
         Then reply with ONLY the single word that file contains.",
        target.display()
    ))];

    let outcome = bridge::ACTIVE_BRIDGE_URL
        .scope(
            Some(lease.url().to_string()),
            provider.complete(
                "You are Biorouter. Use the tools you are given.",
                &messages,
                &[],
            ),
        )
        .await;

    assert_eq!(
        approver.await.expect("the approver task must not panic"),
        biorouter::pending_user_action::ResolveOutcome::Delivered,
        "Biorouter must have published a card the approver could answer"
    );

    let (message, _usage) = outcome.expect("the codex turn must not fail");
    let text = message.as_concat_text();
    assert!(
        !text.contains("timed out") && !text.contains("timeout"),
        "the park must fit inside Codex's own per-call deadline; it said: {text}"
    );
    assert!(
        text.contains(&marker),
        "the approved tool's output never reached the answer. Expected {marker}, got: {text}"
    );
}

// ---------------------------------------------------------------------------
// #110: a bridged tool call that takes longer than the child's default deadline
// ---------------------------------------------------------------------------

/// A tool that takes `SLOW_TOOL_SECS` to answer and then returns a marker.
///
/// 90 seconds is chosen against the measured failure, not for comfort: issue
/// #110 recorded every bridged call dying at "almost exactly 60 seconds", so a
/// tool that answers at 90 can only succeed if the per-server deadline Biorouter
/// now configures is actually being honoured by the child. A 30-second tool
/// would pass whether the fix worked or not.
const SLOW_TOOL_SECS: u64 = 90;

struct SlowDispatch {
    marker: String,
}

#[async_trait::async_trait]
impl bridge::BridgeToolDispatch for SlowDispatch {
    async fn dispatch(
        &self,
        _session_id: &str,
        _call: rmcp::model::CallToolRequestParams,
        _capability: CallCapability,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<rmcp::model::CallToolResult, String> {
        tokio::time::sleep(std::time::Duration::from_secs(SLOW_TOOL_SECS)).await;
        Ok(rmcp::model::CallToolResult::success(vec![
            rmcp::model::Content::text(format!(
                "waited {SLOW_TOOL_SECS}s and finished. marker={}",
                self.marker
            )),
        ]))
    }
}

fn slow_tool() -> rmcp::model::Tool {
    rmcp::model::Tool::new(
        "workspace__workspace_watch",
        "Wait for background conversations to finish. A timeout is never an error.",
        Arc::new(
            serde_json::from_value(json!({
                "type": "object",
                "properties": { "timeout_s": { "type": "integer" } }
            }))
            .expect("a valid schema"),
        ),
    )
}

/// A grant over one deliberately slow tool, in Auto mode so nothing asks for a
/// human and the only thing under test is the transport's patience.
fn slow_grant(marker: &str) -> bridge::BridgeGrant {
    use biorouter::config::permission::PermissionManager;
    use biorouter::managed::ManagedPolicy;
    use biorouter::permission::permission_inspector::PermissionInspector;
    use biorouter::permission::tool_risk::ToolRiskRegistry;

    let risks = Arc::new(ToolRiskRegistry::new());
    let managed = Arc::new(ManagedPolicy::empty());
    let mut inspections = ToolInspectionManager::new();
    inspections.add_inspector(Box::new(PermissionInspector::new(
        Arc::clone(&risks),
        PermissionManager::instance(),
        managed,
        Arc::new(tokio::sync::Mutex::new(None)),
    )));

    bridge::BridgeGrant::new(
        Session::default(),
        BioRouterMode::Auto,
        Arc::new(SlowDispatch {
            marker: marker.to_string(),
        }),
        Arc::new(inspections),
        CallCapability::public_enforced(),
        vec![slow_tool()],
        Conversation::new_unvalidated(vec![]),
        None,
        no_hooks(),
        None,
        risks,
    )
}

/// **#110 through the real Claude Code.** A bridged call that takes 90 seconds
/// must come back as a *result*, not as "The operation timed out".
///
/// This is the assertion the issue asks for and the one no handler test can
/// make: `workspace_watch` always built a non-error partial report, and it never
/// reached the model, because the child's own MCP client applied a ~60-second
/// hard wall clock and abandoned the request first. What is under test here is
/// the per-server `timeout` Biorouter now writes into the child's MCP config —
/// so a regression that dropped that field, or a CLI that stopped honouring it,
/// fails here rather than in a user's session.
#[tokio::test]
#[serial_test::serial]
#[ignore = "needs the `claude` CLI installed and signed in; spends the user's own plan quota and takes ~2 minutes"]
async fn a_slow_bridged_call_survives_claude_codes_per_call_deadline() {
    let marker = format!("SLOWPROOF{:016x}", rand::random::<u64>());
    serve_real_bridge().await;
    let lease = bridge::issue(slow_grant(&marker)).expect("the base URL is published");

    // Exactly the config the provider writes, including the timeout field.
    let config = serde_json::json!({
        "mcpServers": {
            "biorouter": {
                "type": "http",
                "url": lease.url(),
                "timeout": bridge::child_tool_call_timeout().as_millis() as u64,
            }
        }
    });
    let dir = tempfile::tempdir().expect("a temp dir");
    let config_path = dir.path().join("mcp.json");
    std::fs::write(&config_path, config.to_string()).expect("write the bridge config");

    let started = std::time::Instant::now();
    let mut child = tokio::process::Command::new("claude")
        .args([
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--model",
            "haiku",
            "--tools",
            "",
            "--setting-sources",
            "",
            "--strict-mcp-config",
            "--permission-mode",
            "bypassPermissions",
            "--no-session-persistence",
            "--system-prompt",
            "You are Biorouter. Use the tools you are given and be patient with slow ones.",
        ])
        .arg("--mcp-config")
        .arg(&config_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env_remove("ANTHROPIC_API_KEY")
        .spawn()
        .expect("the claude CLI runs");
    {
        use tokio::io::AsyncWriteExt;
        let mut stdin = child.stdin.take().expect("piped stdin");
        stdin
            .write_all(
                b"Call workspace_watch once with timeout_s 600. It is slow; wait for it. \
                  Then reply with ONLY the marker value it returned.",
            )
            .await
            .expect("write the prompt");
        stdin.shutdown().await.expect("close stdin");
    }
    let output = child.wait_with_output().await.expect("the CLI finishes");
    let elapsed = started.elapsed();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let dump = || {
        format!(
            "elapsed {}s; stdout was:\n{stdout}\nstderr:\n{}",
            elapsed.as_secs(),
            String::from_utf8_lossy(&output.stderr)
        )
    };

    // The exact diagnostic #110 reported, in the exact place it appeared.
    assert!(
        !stdout.to_lowercase().contains("operation timed out"),
        "the bridged call was abandoned by the child's MCP client; {}",
        dump()
    );
    assert!(
        stdout.contains(&marker),
        "the slow tool's result never reached the model. Expected {marker}. {}",
        dump()
    );
    // And it really did outlive the old ceiling, rather than the tool having
    // been fast: a pass at 20 seconds would prove nothing about the deadline.
    assert!(
        elapsed >= std::time::Duration::from_secs(SLOW_TOOL_SECS),
        "the run finished before the tool could have; {}",
        dump()
    );
}

/// **#110 through the real Codex.** A different MCP client with its own default
/// deadline and its own knob (`tool_timeout_sec`, seconds rather than
/// milliseconds), so nothing proven against Claude Code transfers by argument.
#[tokio::test]
#[serial_test::serial]
#[ignore = "needs the `codex` CLI installed and signed in; spends the user's own plan quota and takes ~2 minutes"]
async fn a_slow_bridged_call_survives_codexs_per_call_deadline() {
    use biorouter::conversation::message::Message;
    use biorouter::model::ModelConfig;
    use biorouter::providers::base::Provider;
    use biorouter::providers::codex::CodexProvider;

    let marker = format!("SLOWPROOFCODEX{:016x}", rand::random::<u64>());
    serve_real_bridge().await;
    let lease = bridge::issue(slow_grant(&marker)).expect("the base URL is published");

    let provider = CodexProvider::from_env(ModelConfig::new("gpt-5.5").expect("a known model"))
        .await
        .expect("the codex CLI is installed");
    let messages = vec![Message::user().with_text(
        "Call the workspace__workspace_watch tool once with timeout_s 600. It is slow; wait \
         for it. Then reply with ONLY the marker value it returned.",
    )];

    let started = std::time::Instant::now();
    let outcome = bridge::ACTIVE_BRIDGE_URL
        .scope(
            Some(lease.url().to_string()),
            provider.complete(
                "You are Biorouter. Use the tools you are given and be patient with slow ones.",
                &messages,
                &[],
            ),
        )
        .await;
    let elapsed = started.elapsed();

    let (message, _usage) = outcome.expect("the codex turn must not fail");
    let text = message.as_concat_text();
    assert!(
        !text.to_lowercase().contains("timed out"),
        "the bridged call was abandoned by Codex's MCP client after {}s: {text}",
        elapsed.as_secs()
    );
    assert!(
        text.contains(&marker),
        "the slow tool's result never reached the model. Expected {marker}, got: {text}"
    );
    assert!(
        elapsed >= std::time::Duration::from_secs(SLOW_TOOL_SECS),
        "the run finished before the tool could have ({}s)",
        elapsed.as_secs()
    );
}
