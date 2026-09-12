//! The acceptance test: "Generate Workflow from Sessions" must run the same
//! underlying logic as the workflow capability.
//!
//! Capturing a chat as a workflow happens on three surfaces — the desktop's
//! "Make workflow" dialog (via `POST /workflows/create`), the CLI's `/workflow`
//! command, and the model's own `platform__manage_workflow` with
//! `action: "generate"`. The requirement is that the same conversation produces
//! the same document on all three.
//!
//! It did not. The HTTP route enriched a generated workflow with the live
//! session's extensions, knowledge selection and author, inline in the handler;
//! the CLI called the identical `Agent::create_workflow` and saved the raw
//! result. Both compiled, both passed their own tests, and the two YAMLs
//! differed only when somebody compared them — which is why the assertion below
//! is a **diff**, not a pair of independent shape checks.
//!
//! ## Why this is a deterministic test and not a live one
//!
//! A live diff has to run the generator twice and would compare two samples from
//! a non-deterministic model: two runs disagree on wording even when the code is
//! identical, so a live diff can only ever be read by eye. Pinning the model
//! output makes the *deterministic* half — the enrichment, which is the half
//! that actually diverged — the only thing that can move the result.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use biorouter::agents::{Agent, AgentConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::conversation::message::Message;
use biorouter::model::ModelConfig;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::{SessionManager, SessionType};
use biorouter::workflow::service;
use rmcp::model::CallToolRequestParams;
use rmcp::model::Tool;
use tempfile::TempDir;

/// Sandbox this binary's config root before anything resolves `Config::global()`.
///
/// Same reasoning as `tests/agent.rs`: the global config path is a `OnceCell`
/// frozen by whichever test touches it first, so a guard inside one test cannot
/// win the race. Running before `main` is the only placement that always does.
#[ctor::ctor]
fn sandbox_config_root_for_this_test_binary() {
    if std::env::var_os("BIOROUTER_PATH_ROOT").is_some() {
        return;
    }
    let root = TempDir::new().expect("scratch config root");
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    static ROOT: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();
    let _ = ROOT.set(root);
}

/// A provider that always returns the same workflow JSON.
///
/// The generator's own output is not what this test is about — the enrichment
/// applied to it afterwards is — so it is pinned. Anything that differs between
/// the two documents below therefore came from the capture path, which is
/// exactly the fault being tested for.
#[derive(Clone)]
struct FixedWorkflowProvider {
    json: &'static str,
}

const GENERATED_JSON: &str = r#"{
  "title": "Gene association summary",
  "description": "Looks a gene up and summarises its disease associations.",
  "instructions": "Query the graph and report the strongest associations first.",
  "activities": ["Summarise APOE", "Compare two genes"],
  "prompt": "Summarise the disease associations for {{ gene_symbol }}.",
  "parameters": [
    {
      "key": "gene_symbol",
      "input_type": "string",
      "requirement": "user_prompt",
      "description": "HGNC gene symbol"
    }
  ],
  "skills": []
}"#;

#[async_trait]
impl Provider for FixedWorkflowProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new("fixed", "Fixed", "", "fixed-model", vec![], "", vec![])
    }

    fn get_name(&self) -> &str {
        "fixed"
    }

    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        Ok((
            Message::assistant().with_text(self.json),
            ProviderUsage::new("fixed-model".to_string(), Usage::default()),
        ))
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("fixed-model")
    }
}

struct Harness {
    _dir: TempDir,
    agent: Agent,
    session_id: String,
}

async fn harness() -> Harness {
    harness_generating(GENERATED_JSON).await
}

async fn harness_generating(json: &'static str) -> Harness {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().to_path_buf();
    let session_manager = Arc::new(SessionManager::new(data_dir.clone()));
    let permission_manager = Arc::new(PermissionManager::new(data_dir.clone()));
    let agent = Agent::with_config(AgentConfig::new(
        session_manager.clone(),
        permission_manager,
        None,
        biorouter::config::BioRouterMode::Auto,
    ));

    let session = session_manager
        .create_session(
            data_dir.clone(),
            "a chat worth keeping".to_string(),
            SessionType::User,
        )
        .await
        .unwrap();

    agent
        .update_provider(Arc::new(FixedWorkflowProvider { json }), &session.id)
        .await
        .expect("bind the fixed provider");

    // ⚠ The fixture MUST give the enrichment something to do.
    //
    // Without a loaded extension and a knowledge selection, `session_enrichment`
    // returns nothing, `apply_session_enrichment` is a no-op, and the diff below
    // compares two copies of the raw generator output — passing whether or not
    // either path enriches at all. That was measured: deleting the enrichment
    // from the tool path left this test green.
    agent
        .add_extension(biorouter::agents::ExtensionConfig::Platform {
            name: "chatrecall".to_string(),
            // Deliberately self-named, so `enrich_extension_description` has a
            // description to replace. A real one would pass through untouched
            // and that half of the enrichment would go untested.
            description: "chatrecall".to_string(),
            bundled: Some(true),
            available_tools: vec![],
        })
        .await
        .expect("load an extension for the capture to record");

    let knowledge = biorouter_mcp::knowledge::service::KnowledgeService::new_default()
        .expect("a knowledge service in the sandboxed root");
    // Idempotent on purpose. The knowledge root is the process-wide sandbox
    // from the ctor, while each test gets its own session database — and test
    // session ids are minted per database, so both tests here mint the SAME id
    // and reach the same knowledge root. A hard `expect` fails whichever test
    // runs second, which reads as a flake rather than as shared state.
    if let Err(err) = knowledge.create_base("research-kb", "Research", None) {
        assert!(
            err.to_string().contains("already exists"),
            "a base for the capture to record: {err}"
        );
    }
    knowledge
        .set_visible_kbs(
            Some(&session.id),
            &["research-kb".to_string()],
            biorouter_mcp::knowledge::service::PrimaryUpdate::Set("research-kb"),
        )
        .expect("a session selection for the capture to record");

    // A conversation for the generator to read. Persisted, because the tool
    // path reads it back off the session rather than being handed it.
    for message in [
        Message::user().with_text("What diseases is APOE associated with?"),
        Message::assistant().with_text("Here are its strongest associations."),
    ] {
        session_manager
            .add_message(&session.id, &message)
            .await
            .unwrap();
    }

    Harness {
        _dir: dir,
        agent,
        session_id: session.id,
    }
}

/// Strip the fields that legitimately differ between two runs seconds apart.
///
/// Nothing in a workflow is a timestamp today, so this only guards the intent:
/// the comparison is of the DOCUMENT, and a future timestamped field should be
/// excluded here explicitly rather than by weakening the diff.
fn comparable(yaml: &str) -> String {
    yaml.trim().to_string()
}

/// The GUI's capture path and the model's capture path produce the SAME
/// document.
///
/// ⚠ A whole-document diff on purpose. The defect this replaces was not a
/// missing field anybody had thought to assert — it was three fields present on
/// one surface and absent on the other, and every per-field check that existed
/// passed on both. Only comparing the documents catches the next one.
#[tokio::test]
async fn the_gui_and_the_agent_capture_the_same_chat_into_the_same_workflow() {
    let h = harness().await;
    let knowledge = biorouter_mcp::knowledge::service::KnowledgeService::new_default()
        .expect("a knowledge service in the sandboxed root");

    // --- Path A: what `POST /workflows/create` does ---------------------
    let session = h
        .agent
        .config
        .session_manager
        .get_session(&h.session_id, true)
        .await
        .unwrap();
    let mut from_route = h
        .agent
        .create_workflow(session.conversation.clone().unwrap())
        .await
        .expect("the route's generation");
    let enrichment = service::session_enrichment(&h.agent, &knowledge, &h.session_id, None)
        .await
        .expect("the route's enrichment");
    service::apply_session_enrichment(&mut from_route, enrichment);
    let route_yaml = from_route.to_yaml().unwrap();

    // --- Path B: what the model's tool does -----------------------------
    let (_id, result) = h
        .agent
        .dispatch_tool_call(
            CallToolRequestParams {
                task: None,
                meta: None,
                name: "platform__manage_workflow".into(),
                arguments: serde_json::json!({ "action": "generate" })
                    .as_object()
                    .cloned(),
            },
            "req-generate".to_string(),
            None,
            &session,
        )
        .await;
    let content = result
        .expect("the generate action dispatches")
        .result
        .await
        .expect("the generate action succeeds");
    let text = content
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n");

    // The tool returns a preamble and then the YAML; the document starts at the
    // first key the generator always emits.
    let tool_yaml = text
        .split_once("\n\nversion:")
        .map(|(_, rest)| format!("version:{rest}"))
        .unwrap_or_else(|| panic!("the generate result must contain the workflow YAML:\n{text}"));

    assert_eq!(
        comparable(&route_yaml),
        comparable(&tool_yaml),
        "the desktop's capture and the model's capture must produce the same \
         document from the same chat — they diverged once already, on \
         extensions, knowledge_bases and author"
    );

    // And the document is not trivially empty, or the equality above proves
    // nothing.
    assert!(
        route_yaml.contains("Gene association summary"),
        "the generated title must survive both paths: {route_yaml}"
    );
    assert!(
        route_yaml.contains("gene_symbol"),
        "parameters must survive: the generator never emitted them until the \
         prompt asked for them, so a workflow could not be parameterised: {route_yaml}"
    );
    assert!(
        route_yaml.contains("prompt:"),
        "a prompt must survive: without one a generated workflow cannot run \
         headless at all: {route_yaml}"
    );

    // ⚠ The enrichment must have DONE something, or the equality above is a
    // comparison of two identical un-enriched documents and proves nothing.
    // These three fields are exactly the ones that used to appear on the route
    // and not on the other surfaces.
    assert!(
        route_yaml.contains("chatrecall"),
        "the session's extensions must be captured: {route_yaml}"
    );
    assert!(
        route_yaml.contains("research-kb"),
        "the session's knowledge selection must be captured: {route_yaml}"
    );
    assert!(
        !route_yaml.contains("description: chatrecall"),
        "a self-named extension description must be replaced with the canonical \
         one: {route_yaml}"
    );
}

/// The provider pin comes from the SESSION's provider, not the machine default.
///
/// `create_workflow` used to read `Config::global().get_biorouter_provider()`
/// for the name while taking the model name from the bound provider, so a
/// rebound session emitted a provider/model pair that had never coexisted. It
/// also `.expect()`ed, panicking the axum handler on a daemon with no default
/// configured — which is exactly the state this test runs in.
#[tokio::test]
async fn the_settings_pin_names_the_bound_provider_and_never_panics() {
    let h = harness().await;
    let session = h
        .agent
        .config
        .session_manager
        .get_session(&h.session_id, true)
        .await
        .unwrap();

    let workflow = h
        .agent
        .create_workflow(session.conversation.unwrap())
        .await
        .expect("no configured machine default must not panic the capture");

    let settings = workflow.settings.expect("a capture pins its model");
    assert_eq!(
        settings.biorouter_provider.as_deref(),
        Some("fixed"),
        "the pin must name the provider this session is BOUND to"
    );
    assert_eq!(settings.biorouter_model.as_deref(), Some("fixed-model"));
}

/// A generation whose optional parameter the document never refers to.
///
/// Models emit these constantly — `workflow.md` asks for parameters and for
/// `{{ key }}` references separately, and a model that obeys the first half and
/// forgets the second produces exactly this. `output_format` below is declared
/// and then never used.
const GENERATED_JSON_WITH_AN_UNREFERENCED_PARAMETER: &str = r#"{
  "title": "Gene association summary",
  "description": "Looks a gene up and summarises its disease associations.",
  "instructions": "Query the graph and report the strongest associations first.",
  "activities": ["Summarise APOE"],
  "prompt": "Summarise the disease associations for {{ gene_symbol }}.",
  "parameters": [
    {
      "key": "gene_symbol",
      "input_type": "string",
      "requirement": "user_prompt",
      "description": "HGNC gene symbol"
    },
    {
      "key": "output_format",
      "input_type": "select",
      "requirement": "optional",
      "description": "How to format the summary",
      "default": "markdown",
      "options": ["markdown", "table"]
    }
  ],
  "skills": []
}"#;

/// A workflow captured from a chat must be SAVABLE.
///
/// The two parameter validators are individually reasonable and jointly closed
/// over a parameter the document never refers to:
///
///   * `optional` with no `default` → "Optional parameters missing default
///     values in the workflow: output_format."
///   * any requirement, referenced nowhere → "Unnecessary parameter
///     definitions: output_format."
///
/// So there was no `default` — present or absent — for which `POST
/// /workflows/save` accepted the generated document, and "Create workflow from
/// this chat" could not complete at all.
///
/// Measured 2026-09-12. The refusal below, run against `origin/main`'s
/// validator, read exactly `\nUnnecessary parameter definitions:
/// output_format.` and nothing else; posting the same two documents to a
/// running daemon answered **400** both times.
///
/// The generator is the half that is wrong. A parameter nothing refers to is a
/// question the run would ask and then discard, so it is dropped at capture
/// time — next to the existing rule that a parameter list which does not even
/// parse is dropped rather than losing the whole document.
#[tokio::test]
async fn a_captured_workflow_never_declares_a_parameter_the_document_does_not_use() {
    let h = harness_generating(GENERATED_JSON_WITH_AN_UNREFERENCED_PARAMETER).await;
    let knowledge = biorouter_mcp::knowledge::service::KnowledgeService::new_default()
        .expect("a knowledge service in the sandboxed root");
    let session = h
        .agent
        .config
        .session_manager
        .get_session(&h.session_id, true)
        .await
        .unwrap();

    let mut workflow = h
        .agent
        .create_workflow(session.conversation.clone().unwrap())
        .await
        .expect("the capture");
    // The save path validates the ENRICHED document — the same one the modal
    // posts — so the enrichment runs here too.
    let enrichment = service::session_enrichment(&h.agent, &knowledge, &h.session_id, None)
        .await
        .expect("the route's enrichment");
    service::apply_session_enrichment(&mut workflow, enrichment);

    let keys: Vec<String> = workflow
        .parameters
        .iter()
        .flatten()
        .map(|parameter| parameter.key.clone())
        .collect();
    assert_eq!(
        keys,
        vec!["gene_symbol".to_string()],
        "the referenced parameter is kept and the unreferenced one dropped"
    );

    // The whole point: what the capture produced is what `POST /workflows/save`
    // will accept. `service::validate` is the function that route calls.
    service::validate(&workflow).unwrap_or_else(|err| {
        panic!(
            "a workflow captured from a chat must save: {err}\n\n{}",
            workflow.to_yaml().unwrap_or_default()
        )
    });
}

/// The two messages that closed the gap, pinned as a pair.
///
/// Read together they are the specification the generator now satisfies: a
/// parameter must be referenced, and an `optional` one must also carry a
/// default. Neither is relaxed — the "unnecessary" arm is the only thing that
/// catches a key typo (`{{ gene }}` in the prompt beside a parameter keyed
/// `gene_symbol` reports both halves of the mismatch), and a parameter nothing
/// reads is dead weight in a document meant to be shared. What changed is that
/// the message now says what to do about it.
#[test]
fn an_unreferenced_parameter_is_still_refused_and_the_refusal_says_why() {
    let unreferenced_with_a_default = r#"
version: 1.0.0
title: Test
description: Test
instructions: Nothing refers to the parameter below.
parameters:
  - key: output_format
    input_type: string
    requirement: optional
    description: How to format the summary
    default: markdown
"#;
    let err = service::validate(
        &biorouter::workflow::Workflow::from_content(unreferenced_with_a_default).unwrap(),
    )
    .expect_err("a parameter the document never refers to is refused");
    let message = err.to_string();
    assert!(
        message.contains("Unnecessary parameter definitions: output_format."),
        "the refusal still names the parameter: {message}"
    );
    assert!(
        message.contains("{{ output_format }}"),
        "and now says how to fix it, naming the key: {message}"
    );

    let unreferenced_without_a_default = r#"
version: 1.0.0
title: Test
description: Test
instructions: Nothing refers to the parameter below.
parameters:
  - key: output_format
    input_type: string
    requirement: optional
    description: How to format the summary
"#;
    let message = service::validate(
        &biorouter::workflow::Workflow::from_content(unreferenced_without_a_default).unwrap(),
    )
    .expect_err("dropping the default does not help")
    .to_string();
    assert!(
        message.contains("Optional parameters missing default values"),
        "the other half of the pair, unchanged: {message}"
    );
}
