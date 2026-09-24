//! SCOPE-BIND: a Crew grant belongs to the chat it was made to, not to its session id.
//!
//! ⚠ **Security-relevant regressions; a change here needs human review.** Grants are stored
//! by session id, and an id is not one chat. Measured on the QA fixture (2026-09-24):
//! `biorouter run --no-session` minted `<date>_1` in its private store and was refused with
//! "Crew run was revoked" — it had inherited the expired grant of Alice's unrelated
//! `<date>_1`. A deleted chat's id handed to a new chat (a restored backup, a reset database,
//! an older build sharing the file) did the same, and with a live grant the new chat could
//! act under a grant nobody gave it.
//!
//! Each test builds its own store and its own registry, so nothing here reads or writes the
//! process's shared store, except the two that drive the real delete path into the
//! process-wide manager — and those use ids only their own `TempDir` store can mint.

use super::*;
use crate::{
    privacy::{CallCapability, ProviderTier},
    session::{session_manager::SessionType, SessionManager},
};
use tempfile::TempDir;

const CONNECTION: &str = "scope-binding-connection";

fn connection() -> Connection {
    Connection {
        id: CONNECTION.into(),
        node_id: None,
        name: "methods".into(),
        ssh_target: "crew@crew.invalid".into(),
        port: Some(22),
        identity_file: None,
        proxy_jump: None,
        socket_path: "/tmp/scope-binding.sock".into(),
        owner_uid: 10001,
        workspace_id: "workspace-methods".into(),
        workspace_public_key: "11".repeat(32),
        remote_root: None,
        remote_execution: false,
        cluster_connection_id: "cluster-methods".into(),
        mode: ClusterMode::Private,
        institution_id: None,
        policy_epoch: 1,
        status: "disconnected".into(),
        last_error: None,
        device_id: "22".repeat(32),
        public_key: "33".repeat(32),
    }
}

/// A live grant on [`CONNECTION`], bound to `incarnation` (`None`: recorded before grants
/// were bound).
fn grant(run_id: &str, incarnation: Option<i64>) -> Scope {
    Scope {
        connection_id: CONNECTION.into(),
        run_id: run_id.into(),
        channel_id: "channel-methods".into(),
        source_channels: vec!["channel-methods".into()],
        epoch: 1,
        provider_binding: "versa_azure".into(),
        public_provider: false,
        origin_restricted: false,
        institution_ids: BTreeSet::new(),
        institution_policy: true,
        expired: false,
        expires_at: None,
        labels: None,
        session_incarnation: incarnation,
    }
}

fn private_call() -> CallCapability {
    CallCapability::for_test(ProviderTier::Private, true)
}

/// One device: a session store, and a Crew registry that resolves chats against it.
struct Device {
    data: TempDir,
    crew_root: TempDir,
    store: Arc<SessionManager>,
    crew: CrewManager,
}

impl Device {
    async fn new() -> Self {
        let data = TempDir::new().unwrap();
        let crew_root = TempDir::new().unwrap();
        let store = Arc::new(SessionManager::new(data.path().to_path_buf()));
        let crew = CrewManager::new(crew_root.path().to_path_buf()).unwrap();
        crew.use_session_store(store.clone());
        crew.registry.lock().await.connections.push(connection());
        Self {
            data,
            crew_root,
            store,
            crew,
        }
    }

    /// A new saved chat: its id and its incarnation.
    async fn chat(&self) -> (String, i64) {
        let id = self
            .store
            .create_session(
                self.data.path().to_path_buf(),
                "chat".into(),
                SessionType::User,
            )
            .await
            .unwrap()
            .id;
        let incarnation = self.store.session_incarnation(&id).await.unwrap().unwrap();
        (id, incarnation)
    }

    /// Record `scope` under `session` and save the registry, as a grant does.
    async fn record(&self, session: &str, scope: Scope) {
        let mut registry = self.crew.registry.lock().await;
        registry.scopes.insert(session.into(), scope);
        self.crew.persist(&registry).unwrap();
    }

    /// This device's registry as a fresh process loads it.
    fn restarted(&self) -> CrewManager {
        let crew = CrewManager::new(self.crew_root.path().to_path_buf()).unwrap();
        crew.use_session_store(self.store.clone());
        crew
    }

    /// The saved registry's scopes, read from disk.
    fn saved_scopes(&self) -> serde_json::Map<String, Value> {
        let saved: Value = serde_json::from_slice(
            &std::fs::read(self.crew_root.path().join("connections.json")).unwrap(),
        )
        .unwrap();
        saved["scopes"].as_object().cloned().unwrap_or_default()
    }

    /// Delete `session` and hand its id to the next chat, as a store without its
    /// high-water mark does (a restored backup, a reset database, an older build).
    async fn reissue(&self, session: &str) -> (String, i64) {
        self.store.delete_session(session).await.unwrap();
        self.store
            .forget_minted_session_ids_for_test()
            .await
            .unwrap();
        let (id, incarnation) = self.chat().await;
        assert_eq!(
            id, session,
            "the fixture must hand the deleted chat's id to the next chat, or it proves nothing"
        );
        (id, incarnation)
    }
}

/// Everything a genuinely granted chat must still be held to.
async fn assert_restricted_to_its_grant(crew: &CrewManager, session: &str) {
    assert!(crew.is_scoped_session(session).await);
    assert!(crew.scoped_session_ids().await.contains(session));
    assert!(
        crew.authorize_session_tool(session, "developer__shell")
            .await
            .is_err(),
        "a granted chat reached a tool outside Crew"
    );
    crew.authorize_session_tool(session, "crew__request")
        .await
        .unwrap();
}

/// The core defect: a chat handed a granted chat's id after its deletion is not granted.
#[tokio::test]
async fn a_chat_reissued_a_deleted_chats_id_does_not_inherit_its_grant() {
    let device = Device::new().await;
    let (granted, incarnation) = device.chat().await;
    device
        .record(&granted, grant("run-granted", Some(incarnation)))
        .await;
    assert_restricted_to_its_grant(&device.crew, &granted).await;
    device.crew.agent_connections(&granted).await.unwrap();

    // The delete reaches the process-wide registry, not this one: this is the device whose
    // delete hook never ran (another process's registry, or a grant saved before it).
    let (reissued, _) = device.reissue(&granted).await;

    let crew = &device.crew;
    assert!(
        !crew.is_scoped_session(&reissued).await,
        "a new chat inherited the deleted chat's Crew grant"
    );
    assert!(!crew.scoped_session_ids().await.contains(&reissued));
    crew.authorize_session_tool(&reissued, "developer__shell")
        .await
        .expect("the new chat's own tools must not be taken away by another chat's grant");
    crew.check_dispatch(&reissued, &private_call())
        .await
        .expect("the new chat must not be refused under another chat's grant");
    assert!(crew.run_metadata(&reissued).await.is_none());

    // ...and it can never act under that grant.
    let acting = crew.agent_connections(&reissued).await.unwrap_err();
    assert_eq!(acting.to_string(), NO_GRANT);
    let worker = crew
        .worker_request(&reissued, "messages.history", json!({}))
        .await
        .unwrap_err();
    assert_eq!(worker.to_string(), NO_GRANT);
    let revoke = crew.revoke_session(&reissued).await.unwrap_err();
    assert_eq!(revoke.to_string(), NO_GRANT);
    assert_eq!(
        crew.session_grants(CONNECTION).await.unwrap()["grants"],
        json!([])
    );

    // The stale grant is pruned, in memory and on disk.
    assert!(!crew.registry.lock().await.scopes.contains_key(&granted));
    assert!(!device.saved_scopes().contains_key(&granted));
}

/// A live grant is the dangerous half: the reissued chat must not reach the workspace
/// through it, whichever process — and whichever registry — it runs in.
#[tokio::test]
async fn a_reissued_chat_cannot_act_under_a_live_grant_after_a_restart() {
    let device = Device::new().await;
    let (granted, incarnation) = device.chat().await;
    device
        .record(&granted, grant("run-live", Some(incarnation)))
        .await;
    let (reissued, _) = device.reissue(&granted).await;

    let restarted = device.restarted();
    assert!(!restarted.is_scoped_session(&reissued).await);
    assert_eq!(
        restarted
            .agent_connections(&reissued)
            .await
            .unwrap_err()
            .to_string(),
        NO_GRANT
    );
    assert!(!device.saved_scopes().contains_key(&granted));
}

/// A genuinely granted chat keeps its grant, and its restrictions, across a restart.
#[tokio::test]
async fn a_granted_chat_stays_scoped_across_a_restart() {
    let device = Device::new().await;
    let (granted, incarnation) = device.chat().await;
    let (other, _) = device.chat().await;
    device
        .record(&granted, grant("run-granted", Some(incarnation)))
        .await;
    assert_eq!(
        device.saved_scopes()[&granted]["session_incarnation"],
        json!(incarnation),
        "the binding must be saved, or a restart forgets which chat the grant is for"
    );

    let restarted = device.restarted();
    assert_restricted_to_its_grant(&restarted, &granted).await;
    restarted
        .check_dispatch(&granted, &private_call())
        .await
        .unwrap();
    restarted.agent_connections(&granted).await.unwrap();
    assert_eq!(
        restarted.run_metadata(&granted).await.unwrap().run_id,
        "run-granted"
    );
    assert!(!restarted.is_scoped_session(&other).await);
    assert_eq!(
        restarted.scoped_session_ids().await,
        std::collections::HashSet::from([granted.clone()])
    );
}

/// A revoked grant keeps refusing the chat it was made to, before and after a restart:
/// binding a grant to its chat never turns a refusal into "not scoped".
#[tokio::test]
async fn a_revoked_grant_still_refuses_the_same_chat() {
    let device = Device::new().await;
    let (granted, incarnation) = device.chat().await;
    let mut revoked = grant("run-revoked", Some(incarnation));
    revoked.expired = true;
    device.record(&granted, revoked).await;

    for crew in [&device.crew, &device.restarted()] {
        assert_restricted_to_its_grant(crew, &granted).await;
        let refused = crew
            .check_dispatch(&granted, &private_call())
            .await
            .unwrap_err();
        assert_eq!(refused.to_string(), GRANT_REVOKED);
        let worker = crew
            .worker_request(&granted, "messages.history", json!({}))
            .await
            .unwrap_err();
        assert_eq!(worker.to_string(), GRANT_REVOKED);
        assert!(crew.run_metadata(&granted).await.is_some());
    }
}

/// A grant whose chat is gone keeps every restriction but authorizes nothing: a turn still
/// unwinding after the delete may hold Crew context, and must not be let loose with it.
#[tokio::test]
async fn a_deleted_chats_grant_restricts_but_never_authorizes() {
    let device = Device::new().await;
    let (granted, incarnation) = device.chat().await;
    device
        .record(&granted, grant("run-gone", Some(incarnation)))
        .await;
    // Deleted through the store: the process-wide registry is told, this one is not.
    device.store.delete_session(&granted).await.unwrap();

    let crew = &device.crew;
    assert_restricted_to_its_grant(crew, &granted).await;
    for refused in [
        crew.agent_connections(&granted).await.unwrap_err(),
        crew.check_dispatch(&granted, &private_call())
            .await
            .unwrap_err(),
        crew.worker_request(&granted, "messages.history", json!({}))
            .await
            .unwrap_err(),
    ] {
        assert_eq!(refused.to_string(), GRANT_GONE);
    }
    // Still listed and revocable, so the person can end the run at the workspace.
    assert_eq!(
        crew.session_grants(CONNECTION).await.unwrap()["grants"][0]["session_id"],
        json!(granted)
    );
}

/// A store that cannot be read never lifts a restriction and never authorizes.
#[tokio::test]
async fn an_unreadable_store_keeps_the_grant_restrictive() {
    let device = Device::new().await;
    let (granted, incarnation) = device.chat().await;
    device
        .record(&granted, grant("run-granted", Some(incarnation)))
        .await;
    device.store.close().await;

    let crew = &device.crew;
    assert_restricted_to_its_grant(crew, &granted).await;
    for refused in [
        crew.agent_connections(&granted).await.unwrap_err(),
        crew.check_dispatch(&granted, &private_call())
            .await
            .unwrap_err(),
    ] {
        assert_eq!(refused.to_string(), GRANT_UNCONFIRMED);
    }
    let grant_refused = crew.grantable_chat(&granted).await.unwrap_err();
    assert_eq!(grant_refused.to_string(), GRANT_UNCONFIRMED);
    // Nothing was pruned on the strength of a failed read.
    assert!(crew.registry.lock().await.scopes.contains_key(&granted));
    assert!(device.saved_scopes().contains_key(&granted));
}

/// Regression (a): a `--no-session` store mints ids no saved chat can hold, so a grant made
/// to a saved chat never reaches the run — even when the run's store minted first, which is
/// the order that made it `<date>_1`.
#[tokio::test]
async fn a_no_session_store_never_holds_a_saved_chats_grant() {
    let ephemeral_dir = TempDir::new().unwrap();
    let ephemeral = SessionManager::new_ephemeral(ephemeral_dir.path().to_path_buf());
    let mut runs = Vec::new();
    for _ in 0..2 {
        runs.push(
            ephemeral
                .create_session(
                    ephemeral_dir.path().to_path_buf(),
                    "CLI Session".into(),
                    SessionType::Hidden,
                )
                .await
                .unwrap()
                .id,
        );
    }

    let device = Device::new().await;
    let (granted, incarnation) = device.chat().await;
    device
        .record(&granted, grant("run-granted", Some(incarnation)))
        .await;

    assert!(!SessionManager::is_ephemeral_session_id(&granted));
    for run in &runs {
        assert!(
            SessionManager::is_ephemeral_session_id(run),
            "a --no-session store minted `{run}`, an id a saved chat could hold"
        );
        assert_ne!(run, &granted);
        assert!(!device.crew.is_scoped_session(run).await);
        assert!(device.crew.run_metadata(run).await.is_none());
    }
    assert_restricted_to_its_grant(&device.crew, &granted).await;

    // Nor can a run be granted: it is not a chat saved on this device.
    let refused = device.crew.grantable_chat(&runs[0]).await.unwrap_err();
    assert_eq!(refused.to_string(), UNSAVED_CHAT);

    // Two runs never share a namespace either, at once or one after another.
    let other_dir = TempDir::new().unwrap();
    let other = SessionManager::new_ephemeral(other_dir.path().to_path_buf());
    let other_run = other
        .create_session(
            other_dir.path().to_path_buf(),
            "CLI Session".into(),
            SessionType::Hidden,
        )
        .await
        .unwrap()
        .id;
    assert!(SessionManager::is_ephemeral_session_id(&other_run));
    let prefix = |id: &str| id.split_once('_').map(|(prefix, _)| prefix.to_owned());
    assert_ne!(prefix(&other_run), prefix(&runs[0]));
    ephemeral.close().await;
    other.close().await;
}

/// A grant recorded before grants were bound keeps working for the chat that holds its id,
/// and is then bound to that chat, so a later chat under the id cannot inherit it.
#[tokio::test]
async fn a_grant_recorded_before_binding_is_bound_to_the_chat_holding_its_id() {
    let device = Device::new().await;
    let (granted, incarnation) = device.chat().await;
    device.record(&granted, grant("run-legacy", None)).await;

    assert_restricted_to_its_grant(&device.crew, &granted).await;
    device.crew.agent_connections(&granted).await.unwrap();
    assert_eq!(
        device.crew.registry.lock().await.scopes[&granted].session_incarnation,
        Some(incarnation)
    );

    let (reissued, _) = device.reissue(&granted).await;
    assert!(!device.crew.is_scoped_session(&reissued).await);
    assert!(!device.saved_scopes().contains_key(&granted));
}

/// Deleting a chat clears its grant and only its grant: another chat's grant under the same
/// id (another store's) stays, and so does a grant another process saved after this one
/// loaded the registry.
#[tokio::test]
async fn deleting_a_chat_clears_its_own_grant_and_nothing_else() {
    let device = Device::new().await;
    let (granted, incarnation) = device.chat().await;
    let (legacy, _) = device.chat().await;
    device
        .record(&granted, grant("run-granted", Some(incarnation)))
        .await;
    device.record(&legacy, grant("run-legacy", None)).await;
    // Another process's newer grant, on disk only.
    let mut saved: Value = serde_json::from_slice(
        &std::fs::read(device.crew_root.path().join("connections.json")).unwrap(),
    )
    .unwrap();
    saved["scopes"]["elsewhere_1"] = serde_json::to_value(grant("run-elsewhere", Some(7))).unwrap();
    std::fs::write(
        device.crew_root.path().join("connections.json"),
        serde_json::to_vec(&saved).unwrap(),
    )
    .unwrap();
    let own_store = device.store.storage().session_dir().to_path_buf();
    let other_store = device.data.path().join("another-store");

    // Another chat under the same id, in another store: neither grant is its.
    device
        .crew
        .forget_deleted_sessions(
            &[(granted.clone(), incarnation ^ 1), (legacy.clone(), 42)],
            &other_store,
        )
        .await
        .unwrap();
    assert!(device.crew.is_scoped_session(&granted).await);
    assert!(device.saved_scopes().contains_key(&granted));
    assert!(device.saved_scopes().contains_key(&legacy));

    // The chat each grant was made to.
    device
        .crew
        .forget_deleted_sessions(&[(granted.clone(), incarnation)], &other_store)
        .await
        .unwrap();
    device
        .crew
        .forget_deleted_sessions(&[(legacy.clone(), 42)], &own_store)
        .await
        .unwrap();
    let scopes = device.saved_scopes();
    assert!(!scopes.contains_key(&granted) && !scopes.contains_key(&legacy));
    assert!(
        scopes.contains_key("elsewhere_1"),
        "clearing a deleted chat's grant wrote away a grant another process saved"
    );
    assert!(device.crew.registry.lock().await.scopes.is_empty());
}

/// The real delete paths — one chat, and a History reset — reach the process-wide
/// registry and clear the grants of the chats they delete.
#[tokio::test]
async fn the_store_delete_paths_clear_the_grants_of_the_chats_they_delete() {
    let data = TempDir::new().unwrap();
    let store = SessionManager::new(data.path().to_path_buf());
    let mut chats = Vec::new();
    for _ in 0..3 {
        let id = store
            .create_session(data.path().to_path_buf(), "chat".into(), SessionType::User)
            .await
            .unwrap()
            .id;
        let incarnation = store.session_incarnation(&id).await.unwrap().unwrap();
        chats.push((id, incarnation));
    }
    let crew = manager().unwrap();
    {
        let mut registry = crew.registry.lock().await;
        for (index, (id, incarnation)) in chats.iter().enumerate() {
            registry.scopes.insert(
                id.clone(),
                grant(&format!("run-delete-path-{index}"), Some(*incarnation)),
            );
        }
    }
    let held = |id: &str| {
        let crew = crew.clone();
        let id = id.to_owned();
        async move { crew.registry.lock().await.scopes.contains_key(&id) }
    };

    store.delete_session(&chats[0].0).await.unwrap();
    assert!(!held(&chats[0].0).await, "deleting a chat left its grant");
    assert!(held(&chats[1].0).await && held(&chats[2].0).await);

    store.clear_all_sessions().await.unwrap();
    for (id, _) in &chats {
        assert!(!held(id).await, "a History reset left {id}'s grant");
    }
}
