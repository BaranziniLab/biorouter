//! DR-33 at every reasoning effort.
//!
//! At Quick or Deep effort `Agent::provider_with_effort` rebuilds the turn's
//! provider around the language model alone, without the decision-bound
//! wrapper `install_bound_provider` keeps in the slot, so that provider's
//! `tier()` is the model's and not the chat's. These pin that the chat's tier is
//! what the turn is judged on, whatever the effort. The source census beside
//! them is `the_turn_provider_never_decides_a_tier` in
//! `tests/privacy_guard_wiring.rs`.

use super::*;
use crate::agents::effort::ReasoningEffort;
use crate::agents::AgentConfig;
use crate::config::permission::PermissionManager;
use crate::config::BioRouterMode;
use crate::decision::settings::{DECISION_SETTINGS_STATE, DECISION_SETTINGS_VERSION};
use crate::model::ModelConfig;
use crate::privacy::decision_axes::{
    BasisAuthor, DecisionAxes, HostLabel, LoweredBy, NameSource, PublicBasis,
};
use crate::privacy::ModelAffiliation;
use crate::providers::base::{ProviderMetadata, ProviderUsage};
use crate::providers::decision_bound::DecisionBoundProvider;
use crate::providers::provider_binding::{
    AbsoluteCommandPath, SecretFreeEndpoint, VersaAzureCredentialSource,
};
use crate::session::session_manager::{BasisWrite, BasisWriteOutcome, Session, SessionType};
use std::collections::HashMap;
use std::path::PathBuf;
use tempfile::TempDir;

/// One of the two extensions the compiled-in BAAM baseline calls private.
const PRIVATE_EXTENSION: &str = "ucsfomopagent";
const PRIVATE_TOOL: &str = "ucsfomopagent__data_sources";

/// A model stating both privacy axes, and whether it needs a bridge.
struct Model {
    name: &'static str,
    tier: ProviderTier,
    bridge: bool,
}

#[async_trait::async_trait]
impl Provider for Model {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::empty()
    }
    fn get_name(&self) -> &str {
        self.name
    }
    fn tier(&self) -> ProviderTier {
        self.tier
    }
    fn affiliation(&self) -> Option<ModelAffiliation> {
        self.tier.is_private().then_some(ModelAffiliation::Local)
    }
    fn uses_tool_bridge(&self) -> bool {
        self.bridge
    }
    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("gpt-5.5")
    }
    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        Err(ProviderError::Authentication(
            "nothing in this file runs a turn".to_string(),
        ))
    }
}

fn model(name: &'static str, tier: ProviderTier, bridge: bool) -> Arc<dyn Provider> {
    Arc::new(Model { name, tier, bridge })
}

/// The real Versa provider on the UCSF gateway: Private, rebuildable from its
/// own restore binding, which is what a Quick or Deep turn does to it. Nothing
/// here sends it a request.
fn versa() -> Arc<dyn Provider> {
    Arc::new(
        crate::providers::versa_azure::VersaAzureProvider::from_resolved(
            ModelConfig::new_or_fail("gpt-5.5"),
            SecretFreeEndpoint::new(crate::providers::versa_azure::VERSA_AZURE_ENDPOINT.into())
                .unwrap(),
            "gpt-5.5".into(),
            "2025-04-01-preview".into(),
            VersaAzureCredentialSource::ApiKey,
        )
        .expect("Versa on the UCSF gateway"),
    )
}

/// The key `versa()` and its effort rebuilds read, held in memory only.
fn versa_key() -> HashMap<String, String> {
    HashMap::from([(
        "VERSA_AZURE_API_KEY".to_string(),
        "effort-tier-test-key".to_string(),
    )])
}

fn joey() -> LoweredBy {
    LoweredBy {
        source: NameSource::Monitor,
        label: "Done".to_string(),
        model: "joey-1".to_string(),
        host: HostLabel::Baseten,
        others: 0,
    }
}

/// An agent on its own store, with one chat bound to `provider` and the private
/// extension loaded. With `lowered`, the person first gave the chat a public
/// decision model (Baseten's `joey-1`) through the one writer, so the bind
/// installs the decision-bound wrapper: DR-33's Versa plus Baseten.
async fn chat_on(provider: Arc<dyn Provider>, lowered: bool) -> (TempDir, Arc<Agent>, Session) {
    let dir = TempDir::new().unwrap();
    let agent = Arc::new(Agent::with_config(AgentConfig::new(
        Arc::new(SessionManager::new(dir.path().to_path_buf())),
        Arc::new(PermissionManager::new(dir.path().to_path_buf())),
        None,
        BioRouterMode::Auto,
    )));
    let sm = Arc::clone(&agent.config.session_manager);
    let id = sm
        .create_session(PathBuf::from("."), "effort".to_string(), SessionType::User)
        .await
        .unwrap()
        .id;
    if lowered {
        let basis = PublicBasis::new(&id, BasisAuthor::Person, "test", vec![]);
        let outcome = sm
            .write_public_basis(&id, BasisWrite::Set(basis), |data| {
                data.set_extension_state(
                    DECISION_SETTINGS_STATE,
                    DECISION_SETTINGS_VERSION,
                    serde_json::json!({
                        "defaultModel": { "provider": "baseten_decision", "model": "joey-1" },
                        "monitors": [],
                    }),
                );
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(outcome, BasisWriteOutcome::Written(()));
    }
    agent.update_provider(provider, &id).await.unwrap();
    // A real in-process MCP server admitted under a private NAME, so the tier
    // the gates read is stamped by the production admission path.
    agent
        .extension_manager
        .add_inprocess_server(
            PRIVATE_EXTENSION,
            biorouter_mcp::datasql::server::DataSqlServer::new(HashMap::new()),
        )
        .await
        .expect("inject the private extension");
    let session = sm.get_session(&id, false).await.unwrap();
    (dir, agent, session)
}

fn private_call() -> CallToolRequestParams {
    CallToolRequestParams {
        task: None,
        name: PRIVATE_TOOL.into(),
        arguments: Some(rmcp::object!({})),
        meta: None,
    }
}

/// The headline: a private model a public decision model made public is kept
/// away from a private extension at Quick, Normal and Deep effort alike, while
/// the model the chat is bound to stays Private. At Quick and Deep the turn's
/// request provider is a Versa rebuilt WITHOUT the wrapper, whose own tier is
/// Private, so this is the case a gate reading that provider would get wrong.
#[tokio::test]
async fn every_effort_keeps_a_lowered_chat_away_from_a_private_extension() {
    crate::config::with_config_overrides(versa_key(), async {
        let (_dir, agent, session) = chat_on(versa(), true).await;
        let slot = agent
            .bound_provider_unchecked()
            .await
            .expect("a bound model");
        assert!(
            slot.decision_bound_inner().is_some(),
            "precondition: the slot holds the decision-bound wrapper"
        );
        assert_eq!(slot.tier(), ProviderTier::Public);

        for effort in [
            ReasoningEffort::Quick,
            ReasoningEffort::Normal,
            ReasoningEffort::Deep,
        ] {
            let label = effort.as_str();
            // The request provider, built the way `reply_internal` builds it.
            let turn_provider = agent
                .provider_with_effort(effort)
                .await
                .unwrap_or_else(|error| panic!("{label}: {error}"));
            let reply_provider = match &turn_provider {
                Some(provider) => Arc::clone(provider),
                None => agent.provider().await.unwrap(),
            };
            if effort.is_default() {
                assert!(turn_provider.is_none(), "Normal changes nothing");
            } else {
                let rebuilt = turn_provider
                    .as_ref()
                    .unwrap_or_else(|| panic!("{label} must rebuild Versa, or this tests nothing"));
                assert!(
                    rebuilt.decision_bound_inner().is_none(),
                    "{label}: the rebuilt provider has no wrapper"
                );
                assert_eq!(
                    rebuilt.tier(),
                    ProviderTier::Private,
                    "{label}: the rebuilt provider carries the model's own tier"
                );
            }

            // Gate E: the roster prepared for this effort's request provider
            // does not offer the private tool.
            let (tools, ..) = agent
                .prepare_tools_and_prompt_for_provider(
                    &session.id,
                    &session.working_dir,
                    &reply_provider,
                )
                .await
                .unwrap();
            assert!(
                !tools.iter().any(|tool| tool.name.as_ref() == PRIVATE_TOOL),
                "{label}: a lowered chat was offered {PRIVATE_TOOL}"
            );

            // Gate C: the agent loop's dispatch refuses it, in the sentence
            // that names the decision model rather than "switch to a private
            // model", which a Versa chat already runs.
            let (_id, result) = agent
                .dispatch_tool_call(private_call(), format!("req-{label}"), None, &session)
                .await;
            let refusal = match result
                .expect("a dispatch refusal is a tool result")
                .result
                .await
            {
                Ok(_) => panic!("{label}: a lowered chat reached {PRIVATE_EXTENSION}"),
                Err(error) => error.message.to_string(),
            };
            assert!(
                refusal.contains("public decision model"),
                "{label}: {refusal}"
            );
            assert!(refusal.contains(PRIVATE_EXTENSION), "{label}: {refusal}");

            // The model the chat is bound to never moved.
            assert_eq!(
                agent.bound_model_tier().await,
                ProviderTier::Private,
                "{label}"
            );
        }
    })
    .await;
}

/// The control: the same chat with no public decision model reaches the
/// private extension at every effort, so the refusals above are the decision
/// model's doing and not a gate that refuses everything.
#[tokio::test]
async fn every_effort_still_reaches_the_private_extension_without_a_decision_model() {
    crate::config::with_config_overrides(versa_key(), async {
        let (_dir, agent, session) = chat_on(versa(), false).await;
        for effort in [
            ReasoningEffort::Quick,
            ReasoningEffort::Normal,
            ReasoningEffort::Deep,
        ] {
            let label = effort.as_str();
            let turn_provider = agent.provider_with_effort(effort).await.unwrap();
            assert_eq!(turn_provider.is_some(), !effort.is_default(), "{label}");
            let reply_provider = match &turn_provider {
                Some(provider) => Arc::clone(provider),
                None => agent.provider().await.unwrap(),
            };
            let (tools, ..) = agent
                .prepare_tools_and_prompt_for_provider(
                    &session.id,
                    &session.working_dir,
                    &reply_provider,
                )
                .await
                .unwrap();
            assert!(
                tools.iter().any(|tool| tool.name.as_ref() == PRIVATE_TOOL),
                "{label}: a private chat lost {PRIVATE_TOOL}"
            );
            let (_id, result) = agent
                .dispatch_tool_call(private_call(), format!("req-{label}"), None, &session)
                .await;
            result
                .expect("dispatch")
                .result
                .await
                .unwrap_or_else(|error| panic!("{label}: {}", error.message));
        }
    })
    .await;
}

/// The bridge plan's capability is the lesser of the slot's and the request
/// provider's, never wider than either. The request doubles here are
/// bridge-shaped models with a tier the test chooses; every real one is Public
/// (`every_bridge_provider_is_public_so_a_fold_cannot_change_it`), which is
/// exactly why sampling the request alone looked right.
#[tokio::test]
async fn the_bridge_plan_is_never_wider_than_the_chat_or_the_request() {
    struct Row {
        lowered: bool,
        request: ProviderTier,
        expect: ProviderTier,
        lowered_by_decision: bool,
        what: &'static str,
    }
    for row in [
        Row {
            lowered: true,
            request: ProviderTier::Private,
            expect: ProviderTier::Public,
            lowered_by_decision: true,
            what: "a decision model made the chat public; the request's own tier is Private",
        },
        Row {
            lowered: false,
            request: ProviderTier::Public,
            expect: ProviderTier::Public,
            lowered_by_decision: false,
            what: "the slot is Private (a switch while a public coding agent ran)",
        },
        Row {
            lowered: false,
            request: ProviderTier::Private,
            expect: ProviderTier::Private,
            lowered_by_decision: false,
            what: "both Private: nothing to narrow",
        },
    ] {
        let (_dir, agent, session) = chat_on(
            model("versa_azure", ProviderTier::Private, false),
            row.lowered,
        )
        .await;
        let request = model("codex", row.request, true);
        let tools = agent.list_tools(&session.id, None).await;
        let plan = agent
            .prepare_coding_agent_bridge_plan(&session.id, &request, &tools)
            .await
            .expect("a bridge-shaped request has a plan");
        assert_eq!(plan.capability.tier(), row.expect, "{}", row.what);
        assert_eq!(
            plan.capability.lowered_by_decision(),
            row.lowered_by_decision,
            "{}",
            row.what
        );
        if !row.expect.is_private() {
            assert!(
                !plan
                    .tools
                    .iter()
                    .any(|tool| tool.name.as_ref() == PRIVATE_TOOL),
                "{}: the bridge offered {PRIVATE_TOOL}",
                row.what
            );
        }
    }
}

/// The reason `coding_agent_bridge_subagent_context` and `chat_bridge_dispatch`
/// may hand the request provider to readers of its tier: every provider that
/// needs a bridge is Public, and a decision model's fold cannot change a Public
/// model's tier, affiliation or cause. A private bridge provider fails this and
/// must take the slot's provider there first.
#[test]
fn every_bridge_provider_is_public_so_a_fold_cannot_change_it() {
    let command = AbsoluteCommandPath::new(std::env::current_exe().unwrap()).unwrap();
    let claude: Arc<dyn Provider> = Arc::new(
        crate::providers::claude_code::ClaudeCodeProvider::from_resolved(
            ModelConfig::new_or_fail("claude-sonnet-4-6"),
            command.clone(),
        )
        .unwrap(),
    );
    let codex: Arc<dyn Provider> = Arc::new(
        crate::providers::codex::CodexProvider::from_resolved(
            ModelConfig::new_or_fail("gpt-5.5"),
            command,
        )
        .unwrap(),
    );
    // A lead/worker pair needs a bridge when either half does, and is the
    // least of its halves, so a private lead does not make it private. Nothing
    // builds a new pair, but a chat saved as one still restores as one
    // (`factory::create_from_persisted`), so the case stays live.
    let composite: Arc<dyn Provider> =
        Arc::new(crate::providers::lead_worker::LeadWorkerProvider::new(
            model("versa_azure", ProviderTier::Private, false),
            Arc::clone(&codex),
            Some(3),
        ));
    for name in ["claude_code", "codex"] {
        assert!(
            crate::providers::coding_agent::bridge::provider_uses_bridge(name),
            "{name}"
        );
    }
    for provider in [claude, codex, composite] {
        let name = provider.get_name().to_string();
        assert!(provider.uses_tool_bridge(), "{name} needs a bridge");
        assert_eq!(provider.tier(), ProviderTier::Public, "{name}");
        assert_eq!(provider.affiliation(), None, "{name}");
        let folded = DecisionBoundProvider::bind(
            Arc::clone(&provider),
            DecisionAxes::public_for_test(joey()),
        );
        assert_eq!(folded.tier(), provider.tier(), "{name}");
        assert_eq!(folded.affiliation(), provider.affiliation(), "{name}");
        assert!(folded.decision_lowered_by().is_none(), "{name}");
        assert!(folded.uses_tool_bridge(), "{name}");
    }
}
