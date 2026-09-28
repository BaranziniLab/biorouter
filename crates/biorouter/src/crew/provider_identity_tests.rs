//! PROVIDERS-1 and PROVIDERS-2: a Crew grant is bound to the model boundary the person
//! granted, and that boundary is the provider's route and model, not what the provider carries
//! for one turn or learns at run time.
//!
//! ⚠ **Crew provider binding; a change here needs human review.** The binding used to be the
//! hash of the provider's whole restore binding, context window and sampling included. Two
//! ordinary uses moved it with nothing about the destination changing:
//!
//! - Llama Server reads the loaded model's real context window back from the server on its
//!   first request, so a chat granted on `gemma4-12b` (262k in the model table, 128k as the
//!   sidecar allocated it) refused its first answer as it streamed in, and every turn after;
//! - Quick and Deep rebuild the provider with `reasoning_effort` (and, for Quick, a
//!   temperature), so a granted chat refused every turn until effort went back to Normal.
//!
//! Each test checks the old identity would have moved too, so it fails without the fix, and
//! that a different model or route is still refused, so the binding still binds.

use super::*;
use crate::agents::effort::ReasoningEffort;
use crate::model::ModelConfig;
use crate::providers::llamacpp::LlamaCppProvider;
use crate::session::{session_manager::SessionType, SessionManager};
use tempfile::TempDir;

const CONNECTION: &str = "provider-identity-connection";
const REBOUND: &str = "Crew conversation remains bound to its original resolved provider; start a fresh conversation for another model boundary";

/// One device: a session store, a saved chat in it, and a Crew registry that resolves chats
/// against it, with a connection of the privacy mode a grant to `provider` needs.
struct Device {
    _data: TempDir,
    _crew_root: TempDir,
    crew: CrewManager,
    chat: String,
    incarnation: i64,
}

impl Device {
    async fn new(provider: &dyn Provider) -> Self {
        let data = TempDir::new().unwrap();
        let crew_root = TempDir::new().unwrap();
        let store = Arc::new(SessionManager::new(data.path().to_path_buf()));
        let crew = CrewManager::new(crew_root.path().to_path_buf()).unwrap();
        crew.use_session_store(store.clone());
        let chat = store
            .create_session(data.path().to_path_buf(), "chat".into(), SessionType::User)
            .await
            .unwrap()
            .id;
        let incarnation = store.session_incarnation(&chat).await.unwrap().unwrap();
        let public = provider.tier() == ProviderTier::Public;
        crew.registry.lock().await.connections.push(Connection {
            id: CONNECTION.into(),
            node_id: None,
            name: "methods".into(),
            ssh_target: "crew@crew.invalid".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/tmp/provider-identity.sock".into(),
            owner_uid: 10001,
            workspace_id: "workspace-methods".into(),
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: "cluster-methods".into(),
            mode: if public {
                ClusterMode::Public
            } else {
                ClusterMode::Private
            },
            institution_id: None,
            policy_epoch: 1,
            status: "disconnected".into(),
            last_error: None,
            device_id: "22".repeat(32),
            public_key: "33".repeat(32),
        });
        let device = Self {
            _data: data,
            _crew_root: crew_root,
            crew,
            chat,
            incarnation,
        };
        device.grant(provider).await;
        device
    }

    /// Grant the chat access, bound to `provider` as the grant route binds it.
    async fn grant(&self, provider: &dyn Provider) {
        self.crew.registry.lock().await.scopes.insert(
            self.chat.clone(),
            Scope {
                connection_id: CONNECTION.into(),
                run_id: "provider-identity-run".into(),
                channel_id: "channel-methods".into(),
                source_channels: vec!["channel-methods".into()],
                epoch: 1,
                provider_binding: provider_binding(provider),
                public_provider: provider.tier() == ProviderTier::Public,
                origin_restricted: false,
                institution_ids: BTreeSet::new(),
                institution_policy: true,
                expired: false,
                expires_at: None,
                labels: None,
                session_incarnation: Some(self.incarnation),
                session_store: self.crew.own_store(),
                revocation: None,
            },
        );
    }

    async fn check(&self, provider: &dyn Provider) -> Result<()> {
        self.crew.check_provider_binding(&self.chat, provider).await
    }
}

#[tokio::test]
async fn a_llama_server_grant_holds_once_the_server_reports_its_real_window() {
    // `gemma4-12b` is the model a 64 GiB Mac is offered first; the model table gives it
    // 262k, and the sidecar allocates half that.
    let llama = LlamaCppProvider::managed_for_test(ModelConfig::new_or_fail("gemma4-12b"));
    let device = Device::new(&llama).await;
    let before = llama.get_model_config().context_limit;
    let whole_binding = legacy_provider_binding(&llama);
    device.check(&llama).await.unwrap();

    // The first request starts the server, which reports the window it really loaded.
    let loaded = before.map_or(65_536, |window| window / 2);
    llama.observe_live_context_for_test(loaded);
    assert_eq!(llama.get_model_config().context_limit, Some(loaded));
    assert_ne!(
        legacy_provider_binding(&llama),
        whole_binding,
        "the whole binding moved with the window, which is what refused the turn"
    );
    device
        .check(&llama)
        .await
        .expect("the grant still binds the same model on the same server");

    // Another model on the same server is another boundary.
    let other = LlamaCppProvider::managed_for_test(ModelConfig::new_or_fail("gemma4"));
    assert_eq!(device.check(&other).await.unwrap_err().to_string(), REBOUND);
}

/// The effort-stamped provider, rebuilt exactly as `Agent::provider_with_effort` rebuilds it.
async fn with_effort(provider: &dyn Provider, effort: ReasoningEffort) -> Arc<dyn Provider> {
    let mut binding = provider.restore_binding();
    *binding.model_mut() = effort.apply_to_model(binding.model().clone());
    let model = crate::providers::persisted_model_config_from_binding(provider.get_name(), binding)
        .unwrap();
    crate::providers::create_from_persisted(provider.get_name(), model)
        .await
        .unwrap()
}

fn versa(model: &str, deployment: &str) -> Arc<dyn Provider> {
    use crate::providers::provider_binding::{SecretFreeEndpoint, VersaAzureCredentialSource};
    Arc::new(
        crate::providers::versa_azure::VersaAzureProvider::from_resolved(
            ModelConfig::new_or_fail(model),
            SecretFreeEndpoint::new(crate::providers::versa_azure::VERSA_AZURE_ENDPOINT.into())
                .unwrap(),
            deployment.into(),
            "2025-04-01-preview".into(),
            VersaAzureCredentialSource::ApiKey,
        )
        .unwrap(),
    )
}

#[tokio::test]
async fn quick_and_deep_turns_keep_the_grants_model_boundary() {
    crate::config::with_config_overrides(
        std::collections::HashMap::from([(
            "VERSA_AZURE_API_KEY".to_string(),
            "provider-identity-key".to_string(),
        )]),
        async {
            let provider = versa("gpt-5.5", "gpt-5.5");
            let device = Device::new(provider.as_ref()).await;
            for effort in [ReasoningEffort::Quick, ReasoningEffort::Deep] {
                let rebuilt = with_effort(provider.as_ref(), effort).await;
                assert_eq!(
                    rebuilt.restore_binding().model().reasoning_effort,
                    Some(effort),
                    "the rebuilt provider carries the effort"
                );
                assert_ne!(
                    legacy_provider_binding(rebuilt.as_ref()),
                    legacy_provider_binding(provider.as_ref()),
                    "the whole binding moved with the {effort:?} effort, which refused the turn"
                );
                device
                    .check(rebuilt.as_ref())
                    .await
                    .unwrap_or_else(|error| panic!("a {effort:?} turn was refused: {error}"));
            }

            // Another model, or the same model through another deployment, is another
            // boundary, with or without an effort.
            for other in [versa("gpt-5.2", "gpt-5.2"), versa("gpt-5.5", "gpt-5.5-eu")] {
                assert_eq!(
                    device.check(other.as_ref()).await.unwrap_err().to_string(),
                    REBOUND
                );
                let deep = with_effort(other.as_ref(), ReasoningEffort::Deep).await;
                assert_eq!(
                    device.check(deep.as_ref()).await.unwrap_err().to_string(),
                    REBOUND
                );
            }
        },
    )
    .await;
}

/// A grant recorded before the binding left per-turn state out holds the hash of the whole
/// binding. It still binds the provider as it stood, so an upgrade does not refuse a chat
/// granted a moment before it, and it still refuses any other.
#[tokio::test]
async fn a_grant_recorded_under_the_whole_binding_still_binds() {
    let llama = LlamaCppProvider::managed_for_test(ModelConfig::new_or_fail("gemma4-12b"));
    let device = Device::new(&llama).await;
    device
        .crew
        .registry
        .lock()
        .await
        .scopes
        .get_mut(&device.chat)
        .unwrap()
        .provider_binding = legacy_provider_binding(&llama);
    device.check(&llama).await.unwrap();
    let other = LlamaCppProvider::managed_for_test(ModelConfig::new_or_fail("gemma4"));
    assert_eq!(device.check(&other).await.unwrap_err().to_string(), REBOUND);
}

/// The identity is the route, the model and the privacy standing; nothing a turn sets moves
/// it, and each thing that decides where the context goes does.
#[test]
fn the_identity_moves_only_with_what_decides_where_the_context_goes() {
    let base = || LlamaCppProvider::managed_for_test(ModelConfig::new_or_fail("gemma4-12b"));
    let identity = provider_binding(&base());
    let mut per_turn = ModelConfig::new_or_fail("gemma4-12b");
    per_turn.context_limit = Some(8_192);
    per_turn.temperature = Some(0.2);
    per_turn.max_tokens = Some(512);
    per_turn.reasoning_effort = Some(ReasoningEffort::Deep);
    assert_eq!(
        provider_binding(&LlamaCppProvider::managed_for_test(per_turn)),
        identity
    );

    let mut fast = ModelConfig::new_or_fail("gemma4-12b");
    fast.fast_model = Some("gemma4".into());
    let mut toolshim = ModelConfig::new_or_fail("gemma4-12b");
    toolshim.toolshim = true;
    toolshim.toolshim_model = Some("qwen3.6".into());
    for boundary in [ModelConfig::new_or_fail("gemma4"), fast, toolshim] {
        assert_ne!(
            provider_binding(&LlamaCppProvider::managed_for_test(boundary.clone())),
            identity,
            "{boundary:?}"
        );
    }
}
