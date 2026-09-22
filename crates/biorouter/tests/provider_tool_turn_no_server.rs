//! A tool-driven workflow must fail *before* spending a model run when there is
//! no HTTP server for a child coding agent to call back into (#109).
//!
//! # Why this is its own binary
//!
//! The bridge's base URL is process-global — it has to be, because the HTTP
//! handler runs on a different task from the turn that issued the grant. Every
//! other bridge test in this crate publishes one, so a unit test that
//! *unpublished* it would break them by scheduling. A separate integration
//! binary is a separate process that has simply never published a base URL,
//! which is exactly the state under test, and it stays true no matter what any
//! other test does.
//!
//! # Why the refusal matters
//!
//! For a *chat* turn, running the child tool-less is the right degradation: an
//! answer from the conversation beats a failed turn. For a workflow that IS a
//! sequence of tool calls — a knowledge ingest, a scheduled job — it is a
//! guaranteed silent failure. The measured shape (issue #109) is a model that
//! narrates every call as prose, invents its own `<tool_response>OK` replies to
//! continue against, and writes nothing, after a full run the user paid for.

use std::sync::Arc;

/// Sandbox this binary's config root before any test runs, for the reason
/// `tests/agent.rs` gives: `Config::global()` is a one-shot cell resolved the
/// first time anything touches it, so a guard inside a test cannot win the race
/// against whichever test got there first. An outer root wins.
#[ctor::ctor]
fn sandbox_config_root_for_this_test_binary() {
    // An outer root wins only if `Paths` will honour it. `var_os(..).is_some()`
    // also accepted a blank one, which `Paths` reads as absent: this ctor then
    // returned, and every test resolved the developer's real directories.
    if biorouter::config::paths::Paths::path_root_override().is_some() {
        return;
    }
    let root = tempfile::TempDir::new().expect("a scratch config root");
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    // Leaked deliberately: a `static` is never dropped, which is exactly the
    // lifetime the sandbox needs.
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let _ = ROOT.set(root);
}

use async_trait::async_trait;
use rmcp::model::{CallToolRequestParams, CallToolResult, Tool};
use tokio_util::sync::CancellationToken;

use biorouter::conversation::message::Message;
use biorouter::model::ModelConfig;
use biorouter::privacy::CallCapability;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::coding_agent::bridge::BridgeToolDispatch;
use biorouter::providers::errors::ProviderError;
use biorouter::providers::tool_turn::ProviderToolTurnContext;
use biorouter::session::session_manager::Session;

/// A coding-agent-shaped provider that records whether it was called at all.
struct NeverShouldRun {
    called: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl Provider for NeverShouldRun {
    fn metadata() -> ProviderMetadata
    where
        Self: Sized,
    {
        unimplemented!()
    }
    fn get_name(&self) -> &str {
        "codex"
    }
    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("claude-3-5-sonnet-20241022")
    }
    fn uses_tool_bridge(&self) -> bool {
        true
    }
    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.called.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok((
            Message::assistant().with_text("I called every tool, honest"),
            ProviderUsage::new("codex".into(), Usage::new(None, None, None)),
        ))
    }
}

struct NoopDispatch;

#[async_trait]
impl BridgeToolDispatch for NoopDispatch {
    async fn dispatch(
        &self,
        _session_id: &str,
        _call: CallToolRequestParams,
        _capability: CallCapability,
        _cancel: CancellationToken,
    ) -> Result<CallToolResult, String> {
        unreachable!("nothing should reach a dispatcher in this test")
    }
}

#[tokio::test]
async fn a_tool_workflow_refuses_rather_than_running_tool_less() {
    assert!(
        biorouter::providers::coding_agent::bridge::base_url().is_none(),
        "this binary must never publish a base URL; that is the whole fixture"
    );

    let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let provider: Arc<dyn Provider> = Arc::new(NeverShouldRun {
        called: Arc::clone(&called),
    });
    let context = ProviderToolTurnContext::for_workflow(
        Session::default(),
        Arc::new(NoopDispatch),
        CallCapability::public_enforced(),
        None,
    );

    let error = context
        .run(
            &provider,
            "You are Biorouter.",
            &[Message::user().with_text("ingest this")],
            &[],
        )
        .await
        .expect_err("a workflow made of tool calls cannot run tool-less")
        .to_string();

    assert!(
        !called.load(std::sync::atomic::Ordering::SeqCst),
        "the model must not have been called at all — the point is to fail BEFORE \
         spending a run"
    );
    assert!(
        error.contains("codex"),
        "the failure must name the provider it is about: {error}"
    );
    assert!(
        error.contains("biorouterd") || error.contains("desktop app"),
        "the failure must give a precise recovery path: {error}"
    );
}
