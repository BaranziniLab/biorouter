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
//! And the other half, from review: deleting a chat must KEEP its grant. The first fix
//! dropped it on delete, which left a run the workspace still honored with nothing to revoke
//! it through, and let the deleted chat's still-unwinding turn past the Crew-only tool gate.
//!
//! Each test builds its own store and its own registry, so nothing here reads or writes the
//! process's shared store — except the one that drives the daemon's real delete path, which
//! runs in a process of its own.

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
        revocation: None,
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

    /// The directory of the store this device's grants name.
    fn own_store(&self) -> PathBuf {
        self.store.storage().session_dir().to_path_buf()
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
/// unwinding after the delete may hold Crew context, and must not be let loose with it. It
/// stays listed and revocable, because the workspace still honors its run, and it does not
/// read as revoked, because deleting a chat stops nothing there.
#[tokio::test]
async fn a_deleted_chats_grant_restricts_but_never_authorizes() {
    let device = Device::new().await;
    let (bound, bound_incarnation) = device.chat().await;
    let (legacy, legacy_incarnation) = device.chat().await;
    device
        .record(&bound, grant("run-gone", Some(bound_incarnation)))
        .await;
    device.record(&legacy, grant("run-gone-legacy", None)).await;
    // Deleted through the store, which tells the process-wide registry; this device's is
    // told the same way the hook tells that one.
    device.store.delete_session(&bound).await.unwrap();
    device.store.delete_session(&legacy).await.unwrap();
    device
        .crew
        .retire_deleted_sessions(
            &[
                (bound.clone(), bound_incarnation),
                (legacy.clone(), legacy_incarnation),
            ],
            &device.own_store(),
        )
        .await
        .unwrap();

    for crew in [&device.crew, &device.restarted()] {
        for (session, run) in [(&bound, "run-gone"), (&legacy, "run-gone-legacy")] {
            assert_restricted_to_its_grant(crew, session).await;
            for refused in [
                crew.agent_connections(session).await.unwrap_err(),
                crew.check_dispatch(session, &private_call())
                    .await
                    .unwrap_err(),
                crew.worker_request(session, "messages.history", json!({}))
                    .await
                    .unwrap_err(),
            ] {
                assert_eq!(refused.to_string(), GRANT_GONE, "{session}");
            }
            assert_eq!(crew.run_metadata(session).await.unwrap().run_id, run);
        }
        let listed = crew.session_grants(CONNECTION).await.unwrap();
        let listed = listed["grants"].as_array().unwrap();
        let mut sessions: Vec<&str> = listed
            .iter()
            .map(|row| row["session_id"].as_str().unwrap())
            .collect();
        sessions.sort_unstable();
        let mut expected = vec![bound.as_str(), legacy.as_str()];
        expected.sort_unstable();
        assert_eq!(
            sessions, expected,
            "a deleted chat's grant must stay listed, or its run can't be revoked"
        );
        assert!(
            listed.iter().all(|row| row["expired"] == json!(false)),
            "deleting a chat stops nothing at the workspace, so its grant must not read as \
             revoked: {listed:?}"
        );
    }
    // A grant recorded before binding is now bound to the chat it was made to, on disk, so
    // no process reads it as its chat's own with no chat under the id.
    assert_eq!(
        device.saved_scopes()[&legacy]["session_incarnation"],
        json!(legacy_incarnation)
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

/// Deleting a chat binds its grant to it and touches nothing else: a grant another chat
/// under the same id holds (another store's) stays as it was, and so does a grant another
/// process saved after this one loaded the registry. Nothing is removed.
#[tokio::test]
async fn deleting_a_chat_retires_its_own_grant_and_nothing_else() {
    let device = Device::new().await;
    let (granted, incarnation) = device.chat().await;
    let (legacy, legacy_incarnation) = device.chat().await;
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
    device.store.delete_session(&granted).await.unwrap();
    device.store.delete_session(&legacy).await.unwrap();
    let other_store = device.data.path().join("another-store");
    let bindings = |scopes: &serde_json::Map<String, Value>| {
        [granted.as_str(), legacy.as_str(), "elsewhere_1"]
            .map(|session| scopes[session].get("session_incarnation").cloned())
    };

    // Another chat under the same ids, in another store: neither grant is its.
    device
        .crew
        .retire_deleted_sessions(
            &[
                (granted.clone(), incarnation ^ 1),
                (legacy.clone(), legacy_incarnation),
            ],
            &other_store,
        )
        .await
        .unwrap();
    assert_eq!(
        bindings(&device.saved_scopes()),
        [Some(json!(incarnation)), None, Some(json!(7))]
    );
    assert_eq!(
        device.crew.registry.lock().await.scopes[&legacy].session_incarnation,
        None
    );

    // The chats each grant was made to: both kept, the legacy one now bound to its chat.
    device
        .crew
        .retire_deleted_sessions(
            &[
                (granted.clone(), incarnation),
                (legacy.clone(), legacy_incarnation),
            ],
            &device.own_store(),
        )
        .await
        .unwrap();
    let scopes = device.saved_scopes();
    assert_eq!(
        bindings(&scopes),
        [
            Some(json!(incarnation)),
            Some(json!(legacy_incarnation)),
            Some(json!(7))
        ],
        "retiring a deleted chat's grant wrote away a grant another process saved, or \
         bound one it should not have"
    );
    for session in [&granted, &legacy] {
        assert_eq!(scopes[session.as_str()]["expired"], json!(false));
    }
    // In memory: both kept, and — since every write now reads the saved registry back (D8) —
    // the grant another process saved is held here too, as it was saved.
    let registry = device.crew.registry.lock().await;
    let mut held: Vec<&str> = registry.scopes.keys().map(String::as_str).collect();
    held.sort_unstable();
    let mut expected = vec![granted.as_str(), legacy.as_str(), "elsewhere_1"];
    expected.sort_unstable();
    assert_eq!(
        held, expected,
        "a deleted chat's grant was dropped, or another process's grant was not read back"
    );
    assert_eq!(
        registry.scopes[&legacy].session_incarnation,
        Some(legacy_incarnation)
    );
    assert_eq!(registry.scopes["elsewhere_1"].session_incarnation, Some(7));
    assert!(!registry.scopes["elsewhere_1"].expired);
}

/// The daemon's own path, end to end: chats in the process's shared store, deleted through
/// it, checked on the process-wide registry the delete reached — the one the revoke route,
/// the task cancel and every tool gate read. A deleted chat's grant is still there,
/// restricting and revocable; a History reset keeps them the same way; and only a later chat
/// under the id makes one go.
#[tokio::test]
async fn deleting_through_the_store_keeps_the_grant_restricting_and_revocable() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    // File credentials under a profile of this test's own: revoking reads the device key,
    // and a test must never reach the OS keychain.
    let profile = TempDir::new().unwrap();
    let profile_root = profile.path().to_string_lossy().into_owned();
    let _env = crate::test_sandbox::relocate_path_root_and(
        profile_root.as_str(),
        [
            ("BIOROUTER_DEV_PROFILE_ROOT", Some(profile_root.as_str())),
            ("BIOROUTER_DISABLE_KEYRING", Some("true")),
        ],
    );
    let store = SessionManager::instance();
    let mut chats = Vec::new();
    for _ in 0..3 {
        let id = store
            .create_session(
                profile.path().to_path_buf(),
                "chat".into(),
                SessionType::User,
            )
            .await
            .unwrap()
            .id;
        let incarnation = store.session_incarnation(&id).await.unwrap().unwrap();
        chats.push((id, incarnation));
    }
    let [(bound, bound_incarnation), (legacy, legacy_incarnation), (reset, reset_incarnation)] =
        <[(String, i64); 3]>::try_from(chats).unwrap();
    let crew = manager().unwrap();
    {
        let mut registry = crew.registry.lock().await;
        registry.connections.push(connection());
        registry
            .scopes
            .insert(bound.clone(), grant("run-bound", Some(bound_incarnation)));
        // Recorded before grants were bound, and never looked at since, so not bound in
        // memory either: the delete is what has to bind it.
        registry
            .scopes
            .insert(legacy.clone(), grant("run-legacy", None));
        registry
            .scopes
            .insert(reset.clone(), grant("run-reset", Some(reset_incarnation)));
        crew.persist(&registry).unwrap();
    }
    let saved_scopes = || {
        let saved: Value =
            serde_json::from_slice(&std::fs::read(crew.root.join("connections.json")).unwrap())
                .unwrap();
        saved["scopes"].as_object().cloned().unwrap()
    };

    store.delete_session(&bound).await.unwrap();
    store.delete_session(&legacy).await.unwrap();

    for session in [&bound, &legacy] {
        // The unwinding turn stays held to Crew's tools, and cannot act.
        assert_restricted_to_its_grant(&crew, session).await;
        for refused in [
            crew.agent_connections(session).await.unwrap_err(),
            crew.check_dispatch(session, &private_call())
                .await
                .unwrap_err(),
            crew.worker_request(session, "messages.history", json!({}))
                .await
                .unwrap_err(),
        ] {
            assert_eq!(refused.to_string(), GRANT_GONE, "{session}");
        }
        assert!(crew.run_metadata(session).await.is_some());
        assert!(saved_scopes().contains_key(session.as_str()));
    }
    assert_eq!(
        saved_scopes()[&legacy]["session_incarnation"],
        json!(legacy_incarnation),
        "the delete must bind a grant recorded before binding to the chat it was made to"
    );
    let listed = crew.session_grants(CONNECTION).await.unwrap();
    let listed: Vec<&str> = listed["grants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["session_id"].as_str().unwrap())
        .collect();
    assert!(
        listed.contains(&bound.as_str()) && listed.contains(&legacy.as_str()),
        "a deleted chat's grant must stay listed so it can be revoked: {listed:?}"
    );

    // Revocable: the revoke route's call stops it here and asks the workspace...
    let revoked = crew
        .revoke_session_if_current(&bound, "run-bound")
        .await
        .expect("a deleted chat's grant must still be revocable");
    assert!(
        !revoked.remote_confirmed,
        "no workspace answers in this test"
    );
    // ...and so is the deleted task's cancel, which answers a retryable "unconfirmed", not
    // "no grant", so a retry can still reach the workspace.
    let cancelled = crew
        .cancel_run_if_current(&legacy, "run-legacy")
        .await
        .unwrap_err();
    assert!(
        cancelled.downcast_ref::<RevocationUnconfirmed>().is_some(),
        "a deleted task's cancel never reached the workspace: {cancelled}"
    );
    for session in [&bound, &legacy] {
        assert!(crew.registry.lock().await.scopes[session.as_str()].expired);
        assert_eq!(saved_scopes()[session.as_str()]["expired"], json!(true));
        assert_restricted_to_its_grant(&crew, session).await;
    }

    // A History reset keeps its chats' grants the same way.
    store.clear_all_sessions().await.unwrap();
    assert_restricted_to_its_grant(&crew, &reset).await;
    assert_eq!(
        crew.agent_connections(&reset)
            .await
            .unwrap_err()
            .to_string(),
        GRANT_GONE
    );

    // Only a later chat under the id shows a grant was an earlier chat's, and prunes it.
    store.forget_minted_session_ids_for_test().await.unwrap();
    for session in [&bound, &legacy] {
        let reissued = store
            .create_session(
                profile.path().to_path_buf(),
                "chat".into(),
                SessionType::User,
            )
            .await
            .unwrap()
            .id;
        assert_eq!(
            &reissued, session,
            "the fixture must hand the deleted chat's id to the next chat, or it proves nothing"
        );
        assert!(
            !crew.is_scoped_session(session).await,
            "a new chat inherited a deleted chat's Crew grant"
        );
        crew.authorize_session_tool(session, "developer__shell")
            .await
            .unwrap();
        assert!(!crew.registry.lock().await.scopes.contains_key(session));
        assert!(!saved_scopes().contains_key(session.as_str()));
    }
    assert!(crew.is_scoped_session(&reset).await);
}
