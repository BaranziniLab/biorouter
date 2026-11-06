//! A chat's `crew__read grants_list` (account ownership AO-12) and the names a grant carries.
//!
//! A grant's `labels` name the workspace's channels, teams and people: the names the person saw
//! when granting. Those are workspace content, so a chat is shown them only when it may read the
//! workspace and the person let it read it as them, and never when the workspace can't be read
//! to say so. What is pinned here, against the real host, the real `CrewManager` and a grant
//! registry seeded before the manager first reads it:
//!
//! - a public model listing a Private workspace's grants gets the rows without their labels,
//!   counted as withheld, and that includes a replaced grant whose chat ID now names its own
//!   chat;
//! - a private model gets none either while the workspace can't be read (here it is not
//!   connected), even with the person's consent: the check fails closed;
//! - everything else the listing says (chats, runs, consents) is as before.
//!
//! ⚠ **Its own binary**: the grant registry is loaded once per process, the first time Crew is
//! used, so a seeded grant has to be on disk before anything else touches Crew, and the host is
//! a process-global `OnceLock`. Offline: the seeded connection is never connected, so nothing
//! here opens a socket.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;

use biorouter::config::paths::Paths;
use biorouter::crew::tool_host::{self, PersonTool, PersonToolCall};
use biorouter::pending_user_action::PendingUserActions;
use biorouter::privacy::affiliation::ModelAffiliation;
use biorouter::privacy::{CallCapability, ProviderTier};
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage};
use biorouter::session::session_manager::SessionType;
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

/// The user-action key the desktop app would hold; the daemon keeps only its digest.
const USER_KEY: &str = "crew-tools-grants-user-action-key";
/// The saved connection every grant here is on: Private, `ucsf`, never connected.
const CONNECTION: &str = "conn-grants-lab";
/// The run of the chat's live grant, and of the earlier one it replaced.
const RUN: &str = "run-grants-live";
const EARLIER_RUN: &str = "run-grants-earlier";
/// Names only the workspace's own members may read.
const CHANNEL_NAME: &str = "#hiv-cohort";
const TEAM_NAME: &str = "HIV Cohort Study";
const PERSON_NAME: &str = "Alice Chen (@alice)";

/// Crew's development credential backend: keys are files, never the OS keychain.
#[ctor::ctor]
fn select_the_file_credential_backend() {
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    if std::env::var_os("BIOROUTER_DEV_PROFILE_ROOT").is_none() {
        std::env::set_var(
            "BIOROUTER_DEV_PROFILE_ROOT",
            std::env::temp_dir().join(format!(
                "biorouter-crew-tools-grants-profile-{}",
                std::process::id()
            )),
        );
    }
}

/// The daemon's state and a person's own chat holding a grant, seeded once, before anything
/// reads Crew.
static FIXTURE: OnceCell<(Arc<AppState>, String)> = OnceCell::const_new();

async fn fixture() -> (Arc<AppState>, String) {
    FIXTURE
        .get_or_init(|| async {
            let digest: [u8; 32] =
                <sha2::Sha256 as sha2::Digest>::digest(USER_KEY.as_bytes()).into();
            biorouter_server::auth::install_user_action_digest(Some(digest));
            let state = AppState::new().await.expect("app state");
            biorouter_server::routes::crew_tools::install(state.clone());
            let chat = state
                .session_manager()
                .create_session(
                    std::env::temp_dir().join("crew-tools-grants"),
                    "a chat with Crew access".to_owned(),
                    SessionType::User,
                )
                .await
                .expect("a chat")
                .id;
            seed(&chat);
            (state, chat)
        })
        .await
        .clone()
}

/// A grant as the registry keeps it, with the names the person saw when granting.
fn scope(run: &str) -> Value {
    json!({
        "connection_id": CONNECTION,
        "run_id": run,
        "channel_id": "chan-cohort",
        "source_channels": ["chan-cohort"],
        "epoch": 1,
        "provider_binding": "versa_azure",
        "public_provider": false,
        "expired": false,
        "labels": {
            "you": PERSON_NAME,
            "workspace": "lab",
            "destination": {"channel_id": "chan-cohort", "label": CHANNEL_NAME, "team": TEAM_NAME},
            "sources": [{"channel_id": "chan-cohort", "label": CHANNEL_NAME, "team": TEAM_NAME}],
        },
    })
}

fn seed(chat: &str) {
    let crew_root = Paths::config_dir().join("crew");
    std::fs::create_dir_all(&crew_root).unwrap();
    assert!(
        !crew_root.join("connections.json").exists(),
        "something saved Crew connections before the fixture"
    );
    let mut earlier = scope(EARLIER_RUN);
    earlier["expired"] = json!(true);
    earlier["revocation"] = json!("unconfirmed");
    let registry = json!({
        "connections": [{
            "id": CONNECTION,
            "name": "lab",
            "ssh_target": "crew@crew.invalid",
            "port": null,
            "identity_file": null,
            "proxy_jump": null,
            "socket_path": "/tmp/crew-tools-grants.sock",
            "owner_uid": 501,
            "workspace_id": "workspace-grants",
            "workspace_public_key": "00".repeat(32),
            "cluster_connection_id": "cluster-grants",
            "mode": "private",
            "institution_id": "ucsf",
            "policy_epoch": 1,
            "status": "disconnected",
            "last_error": null,
            "device_id": "11".repeat(32),
            "public_key": "22".repeat(32),
        }],
        "scopes": {chat: scope(RUN)},
        "replaced": [{"session_id": chat, "scope": earlier}],
    });
    std::fs::write(
        crew_root.join("connections.json"),
        serde_json::to_vec(&registry).unwrap(),
    )
    .unwrap();
}

struct Model;

#[async_trait::async_trait]
impl Provider for Model {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        "crew-tools-grants-fixture"
    }

    fn get_model_config(&self) -> biorouter::model::ModelConfig {
        biorouter::model::ModelConfig::new_or_fail("synthetic")
    }

    fn tier(&self) -> ProviderTier {
        ProviderTier::Private
    }

    fn affiliation(&self) -> Option<ModelAffiliation> {
        Some(ModelAffiliation::Local)
    }

    async fn complete_with_model(
        &self,
        _: &biorouter::model::ModelConfig,
        _: &str,
        _: &[biorouter::conversation::message::Message],
        _: &[rmcp::model::Tool],
    ) -> Result<
        (biorouter::conversation::message::Message, ProviderUsage),
        biorouter::providers::errors::ProviderError,
    > {
        unreachable!("the person tools never ask a model")
    }
}

/// A model on this machine: Private, and compatible with every institution.
async fn private_local() -> CallCapability {
    let provider: biorouter::agents::types::SharedProvider =
        Arc::new(tokio::sync::Mutex::new(Some(Arc::new(Model))));
    CallCapability::sample(&provider).await
}

/// `grants_list` on the seeded connection, from `session` on `capability`. It parks no card.
async fn grants_list(session: &str, capability: CallCapability) -> Value {
    let host = tool_host::get().expect("the host is installed");
    host.call(PersonToolCall {
        tool: PersonTool::Read,
        action: "grants_list".to_owned(),
        // By its name: the resolver takes an ID only in a saved connection's UUID form.
        params: json!({"connection": "lab"}),
        session_id: session.to_owned(),
        tool_call_id: None,
        capability,
        cancel: CancellationToken::new(),
        approvals: Arc::clone(PendingUserActions::global()),
    })
    .await
    .unwrap_or_else(|error| panic!("grants_list refused: {error}"))
}

/// No name the person saw when granting reaches the answer.
fn names_nothing(answer: &Value) {
    let text = answer.to_string();
    for name in [CHANNEL_NAME, TEAM_NAME, PERSON_NAME, "hiv-cohort"] {
        assert!(!text.contains(name), "{name} reached the model: {text}");
    }
}

/// A public model gets the grants on a Private workspace without their names, the replaced one
/// included, and is told how many were left out. A private model the person let read the
/// workspace still gets no names while the workspace can't be read to say whether it may: the
/// labels are left out, not shown on trust. One test, because the fixture's daemon state
/// belongs to the runtime that made it.
#[tokio::test(flavor = "multi_thread")]
async fn a_grant_s_names_reach_only_a_call_that_may_read_the_workspace() {
    let (state, chat) = fixture().await;
    let saved = biorouter::crew::manager()
        .expect("the sandboxed Crew manager")
        .list()
        .await;
    assert!(
        saved.iter().any(|connection| connection.id == CONNECTION),
        "the seeded connection did not load: {:?}",
        saved.iter().map(|c| &c.id).collect::<Vec<_>>()
    );
    let answer = grants_list(&chat, CallCapability::public_enforced()).await;
    let grants = answer["grants"].as_array().expect("grants");
    assert_eq!(grants.len(), 1, "{answer}");
    assert_eq!(grants[0]["run_id"], RUN);
    assert!(grants[0]["labels"].is_null(), "{answer}");
    let replaced = answer["replaced_grants"]
        .as_array()
        .expect("replaced grants");
    assert_eq!(replaced.len(), 1, "{answer}");
    assert!(replaced[0]["labels"].is_null(), "{answer}");
    assert_eq!(answer["withheld"]["labels"], 2, "{answer}");
    names_nothing(&answer);

    let crew = biorouter::crew::manager().expect("the sandboxed Crew manager");
    let asker = state
        .session_manager()
        .create_session(
            std::env::temp_dir().join("crew-tools-grants"),
            "an asker".to_owned(),
            SessionType::User,
        )
        .await
        .expect("a chat")
        .id;
    crew.grant_person_read(&asker, CONNECTION)
        .await
        .expect("the consent saves");
    let answer = grants_list(&asker, private_local().await).await;
    let grants = answer["grants"].as_array().expect("grants");
    assert!(
        grants
            .iter()
            .any(|grant| grant["session_id"] == chat.as_str()),
        "{answer}"
    );
    assert!(
        grants.iter().all(|grant| grant["labels"].is_null()),
        "{answer}"
    );
    assert!(
        answer["person_reads"].as_array().is_some_and(|reads| reads
            .iter()
            .any(|read| read["session_id"] == asker.as_str())),
        "{answer}"
    );
    names_nothing(&answer);
}
