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
        remote_access: Default::default(),
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
/// were bound), with no store recorded (as for a grant recorded before stores were kept).
/// [`Device::granted`] is a grant made now.
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
        session_store: None,
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

    /// A live grant made now to a chat of this device's store: bound to the chat and to the
    /// store, as the grant path records it (CROSSCUT-8).
    fn granted(&self, run_id: &str, incarnation: i64) -> Scope {
        Scope {
            session_store: Some(store_name(&self.own_store())),
            ..grant(run_id, Some(incarnation))
        }
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
        .record(&granted, device.granted("run-granted", incarnation))
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
        .record(&granted, device.granted("run-live", incarnation))
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
        .record(&granted, device.granted("run-granted", incarnation))
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
    let mut revoked = device.granted("run-revoked", incarnation);
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
        .record(&bound, device.granted("run-gone", bound_incarnation))
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
    // And to the store that saw the chat go (CROSSCUT-8), the one that may later forget it.
    assert_eq!(
        device.saved_scopes()[&legacy]["session_store"],
        json!(store_name(&device.own_store()))
    );
}

/// A store that cannot be read never lifts a restriction and never authorizes.
#[tokio::test]
async fn an_unreadable_store_keeps_the_grant_restrictive() {
    let device = Device::new().await;
    let (granted, incarnation) = device.chat().await;
    device
        .record(&granted, device.granted("run-granted", incarnation))
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
        .record(&granted, device.granted("run-granted", incarnation))
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
        .record(&granted, device.granted("run-granted", incarnation))
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
    assert!(
        device.saved_scopes()[&legacy]
            .get("session_store")
            .is_none(),
        "another store's delete recorded itself as the store of a grant it never held"
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
        assert_eq!(
            scopes[session.as_str()]["session_store"],
            json!(store_name(&device.own_store())),
            "{session}"
        );
    }
    assert!(scopes["elsewhere_1"].get("session_store").is_none());
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
        registry.scopes.insert(
            bound.clone(),
            Scope {
                session_store: crew.own_store(),
                ..grant("run-bound", Some(bound_incarnation))
            },
        );
        // Recorded before grants were bound, and never looked at since, so not bound in
        // memory either: the delete is what has to bind it.
        registry
            .scopes
            .insert(legacy.clone(), grant("run-legacy", None));
        registry.scopes.insert(
            reset.clone(),
            Scope {
                session_store: crew.own_store(),
                ..grant("run-reset", Some(reset_incarnation))
            },
        );
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
    assert_eq!(
        saved_scopes()[&legacy]["session_store"],
        json!(crew.own_store()),
        "the delete must record the store it saw the chat go from"
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

// -------------------------------------------------------------------------------------------------
// Account ownership: removed connections' records (AO-11), the scoped tool allowlist and read
// consent (AO-12), new devices (AO-10)
// -------------------------------------------------------------------------------------------------

/// File credentials under a profile of this test's own, so nothing reaches the OS keychain.
/// Only in a process of its own.
fn file_credentials(profile: &Path) -> env_lock::EnvGuard<'static> {
    let profile_root = profile.to_string_lossy().into_owned();
    crate::test_sandbox::relocate_path_root_and(
        profile_root.as_str(),
        [
            ("BIOROUTER_DEV_PROFILE_ROOT", Some(profile_root.as_str())),
            ("BIOROUTER_DISABLE_KEYRING", Some("true")),
        ],
    )
}

/// A stopped grant whose run ended long enough ago to have settled.
fn settled(scope: Scope) -> Scope {
    Scope {
        expired: true,
        expires_at: Some(revocation::unix_now() - revocation::REPLACED_KEPT_PAST_END - 60),
        ..scope
    }
}

/// AO-11: a removed connection's grant goes, with its run credential, only once its chat is
/// gone from this store and the grant has settled. A grant whose chat still exists is that
/// chat's restriction and stays; so does one of another store, one not yet settled and one
/// never bound to its chat. Finished preparations and read consents of gone connections go in
/// the same sweep.
#[tokio::test]
async fn a_removed_connections_grant_is_swept_only_once_its_chat_is_gone_and_settled() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let profile = TempDir::new().unwrap();
    let _env = file_credentials(profile.path());
    let device = Device::new().await;
    let crew = &device.crew;
    let (live, live_incarnation) = device.chat().await;
    let gone = "gone-chat";
    device
        .record(gone, settled(device.granted("run-gone", 4242)))
        .await;
    device
        .record(&live, settled(device.granted("run-live", live_incarnation)))
        .await;
    device
        .record(
            "elsewhere-chat",
            Scope {
                session_store: Some("/another/store".into()),
                ..settled(device.granted("run-elsewhere", 7))
            },
        )
        .await;
    device
        .record(
            "unsettled-chat",
            Scope {
                expires_at: Some(revocation::unix_now()),
                ..settled(device.granted("run-unsettled", 8))
            },
        )
        .await;
    device
        .record(
            "unbound-chat",
            Scope {
                session_incarnation: None,
                ..settled(device.granted("run-unbound", 9))
            },
        )
        .await;
    for session in [gone, live.as_str()] {
        crew.write_credential(&format!("run:{session}"), "run-credential")
            .unwrap();
    }
    crew.grant_person_read(&live, CONNECTION).await.unwrap();
    {
        let mut registry = crew.registry.lock().await;
        registry
            .completed_preparations
            .insert(CONNECTION.into(), "saved".into());
        registry
            .completed_preparations
            .insert("gone-preparation".into(), "gone".into());
        crew.persist(&registry).unwrap();
    }

    // While the connection is saved, its grants all stay; a gone preparation does not.
    crew.sweep_removed_connections().await;
    assert_eq!(device.saved_scopes().len(), 5);
    assert_eq!(
        crew.registry
            .lock()
            .await
            .completed_preparations
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec![CONNECTION.to_owned()]
    );

    // The connection is removed (as another process saved it).
    {
        let mut registry = crew.registry.lock().await;
        registry.connections.clear();
        crew.persist(&registry).unwrap();
    }
    crew.sweep_removed_connections().await;
    let saved = device.saved_scopes();
    assert!(
        !saved.contains_key(gone),
        "the gone chat's settled grant goes"
    );
    assert!(crew.read_credential(&format!("run:{gone}")).is_err());
    for kept in [
        live.as_str(),
        "elsewhere-chat",
        "unsettled-chat",
        "unbound-chat",
    ] {
        assert!(saved.contains_key(kept), "{kept} was swept");
    }
    assert!(crew.read_credential(&format!("run:{live}")).is_ok());
    let registry = crew.registry.lock().await;
    assert!(registry.completed_preparations.is_empty());
    assert!(registry.person_reads.is_empty());
}

/// AO-11 fails closed: a store that cannot answer, or a saved registry that cannot be read,
/// prunes nothing.
#[tokio::test]
async fn a_sweep_prunes_nothing_when_the_store_or_the_registry_cannot_answer() {
    let device = Device::new().await;
    device
        .record("gone-chat", settled(device.granted("run-gone", 4242)))
        .await;
    {
        let mut registry = device.crew.registry.lock().await;
        registry.connections.clear();
        device.crew.persist(&registry).unwrap();
    }
    device.store.close().await;
    device.crew.sweep_removed_connections().await;
    assert!(device.saved_scopes().contains_key("gone-chat"));

    let unreadable = Device::new().await;
    unreadable
        .record("gone-chat", settled(unreadable.granted("run-gone", 4242)))
        .await;
    {
        let mut registry = unreadable.crew.registry.lock().await;
        registry.connections.clear();
        unreadable.crew.persist(&registry).unwrap();
    }
    let path = unreadable.crew_root.path().join("connections.json");
    let broken = b"{\"connections\": [{\"unreadable\": tru";
    std::fs::write(&path, broken).unwrap();
    unreadable.crew.sweep_removed_connections().await;
    assert_eq!(std::fs::read(&path).unwrap(), broken);
    assert!(unreadable
        .crew
        .registry
        .lock()
        .await
        .scopes
        .contains_key("gone-chat"));
}

/// AO-12, gate B-1: a chat a grant restricts may call exactly the grant's two worker tools and
/// the checklist; the person tools are refused there.
#[tokio::test]
async fn a_scoped_chat_may_call_only_the_worker_tools_and_the_checklist() {
    let device = Device::new().await;
    let (chat, incarnation) = device.chat().await;
    device
        .record(&chat, device.granted("run-scoped", incarnation))
        .await;
    let crew = &device.crew;
    for allowed in ["crew__connections", "crew__request", "todo__write"] {
        crew.authorize_session_tool(&chat, allowed).await.unwrap();
    }
    for refused in [
        "crew__read",
        "crew__change",
        "crew__anything",
        "developer__shell",
    ] {
        assert!(
            crew.authorize_session_tool(&chat, refused).await.is_err(),
            "{refused}"
        );
    }
    let (unscoped, _) = device.chat().await;
    crew.authorize_session_tool(&unscoped, "crew__change")
        .await
        .unwrap();
}

/// AO-12: a chat's consent to read a workspace as the person is kept per chat and connection,
/// in the saved registry, listed and revocable, and never inherited by a later chat under the
/// same id.
#[tokio::test]
async fn a_read_consent_is_kept_per_chat_and_connection_and_never_inherited() {
    let device = Device::new().await;
    let (chat, _) = device.chat().await;
    let crew = &device.crew;
    assert!(!crew.has_person_read(&chat, CONNECTION).await);
    crew.grant_person_read(&chat, CONNECTION).await.unwrap();
    assert!(crew.has_person_read(&chat, CONNECTION).await);
    assert!(!crew.has_person_read(&chat, "another-connection").await);
    assert_eq!(crew.person_reads(&chat).await.len(), 1);
    assert!(
        device.restarted().has_person_read(&chat, CONNECTION).await,
        "another process reads the same consent"
    );
    assert_eq!(
        crew.grant_person_read(&chat, "another-connection")
            .await
            .unwrap_err()
            .to_string(),
        "Unknown Crew connection"
    );
    assert_eq!(
        crew.grant_person_read("never-saved", CONNECTION)
            .await
            .unwrap_err()
            .to_string(),
        UNSAVED_CHAT
    );

    assert!(crew.revoke_person_read(&chat, CONNECTION).await.unwrap());
    assert!(!crew.has_person_read(&chat, CONNECTION).await);
    assert!(!crew.revoke_person_read(&chat, CONNECTION).await.unwrap());

    // A chat handed the id of a chat that consented does not inherit the consent.
    crew.grant_person_read(&chat, CONNECTION).await.unwrap();
    let rows = crew.person_read_rows().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session_id, chat);
    let (reissued, _) = device.reissue(&chat).await;
    assert!(!crew.has_person_read(&reissued, CONNECTION).await);
    assert!(crew.person_reads(&reissued).await.is_empty());
}

/// AO-10 through the manager: the first snapshot sets what was seen; a device added after it
/// is reported, in every process sharing the profile, until the person acknowledges it.
#[tokio::test]
async fn a_new_device_is_reported_until_acknowledged() {
    let device = Device::new().await;
    let crew = &device.crew;
    // Saved, so a second process reads what this one sees.
    {
        let registry = crew.registry.lock().await;
        crew.persist(&registry).unwrap();
    }
    let devices = |listed: &[(&str, &str, u64)]| {
        let listed: Vec<Value> = listed
            .iter()
            .map(|(fingerprint, via, at)| {
                json!({"fingerprint": fingerprint, "added_via": via, "added_at": at})
            })
            .collect();
        json!({"actor": {"id": "p-1", "devices": listed}})
    };
    // This computer's own device, as the snapshot shows it.
    let own = "2222 2222 2222 2222";
    let laptop = "ABCD ABCD ABCD ABCD";
    let desktop = "1234 5678 9ABC DEF0";
    crew.observe_devices(
        CONNECTION,
        &devices(&[(own, "invitation", 1), (laptop, "invitation", 2)]),
    )
    .await;
    assert!(crew.new_devices(CONNECTION).await.unwrap().is_empty());

    let added = devices(&[
        (own, "invitation", 1),
        (laptop, "invitation", 2),
        (desktop, "account", 3),
    ]);
    crew.observe_devices(CONNECTION, &added).await;
    let reported = crew.new_devices(CONNECTION).await.unwrap();
    assert_eq!(
        reported,
        vec![NewDevice {
            fingerprint: desktop.into(),
            added_via: Some("account".into()),
            added_at: Some(3),
        }]
    );
    assert_eq!(
        device.restarted().new_devices(CONNECTION).await.unwrap(),
        reported
    );

    // Another device appears after the person was shown the list: acknowledging what they
    // were shown leaves it reported.
    let phone = "5555 6666 7777 8888";
    let more = devices(&[
        (own, "invitation", 1),
        (laptop, "invitation", 2),
        (desktop, "account", 3),
        (phone, "account", 4),
    ]);
    crew.observe_devices(CONNECTION, &more).await;
    let shown: Vec<String> = reported.iter().map(|d| d.fingerprint.clone()).collect();
    crew.acknowledge_devices(CONNECTION, &shown).await.unwrap();
    crew.observe_devices(CONNECTION, &more).await;
    let left = crew.new_devices(CONNECTION).await.unwrap();
    assert_eq!(
        left.iter()
            .map(|d| d.fingerprint.as_str())
            .collect::<Vec<_>>(),
        [phone]
    );
    assert_eq!(
        device.restarted().new_devices(CONNECTION).await.unwrap(),
        left
    );

    crew.acknowledge_devices(CONNECTION, &[phone.to_owned()])
        .await
        .unwrap();
    crew.observe_devices(CONNECTION, &more).await;
    assert!(crew.new_devices(CONNECTION).await.unwrap().is_empty());
    assert!(device
        .restarted()
        .new_devices(CONNECTION)
        .await
        .unwrap()
        .is_empty());
    assert!(crew.new_devices("not-a-connection").await.is_err());
    assert!(crew
        .acknowledge_devices("not-a-connection", &shown)
        .await
        .is_err());
}
