//! D8: several processes share one Crew registry, and none may write another's changes away.
//!
//! ⚠ **Grant and revocation state; a change here needs human review.** Every process that
//! opens a profile — the desktop's daemon, a CLI run, a second daemon — loads
//! `connections.json` once and keeps its own copy. Every save used to write that whole copy
//! back, so a process that had loaded before another's change put the old state back: a
//! connection the other saved vanished, and a grant the other revoked came back live. Each
//! test here opens two managers on one directory, which is exactly two processes' view of it:
//! each holds its own copy and its own handle on `connections.lock`.

use super::*;
#[cfg(unix)]
use crate::privacy::{CallCapability, ProviderTier};
use crate::session::SessionManager;
use std::fs;
use tempfile::TempDir;

const FIRST: &str = "31313131-3131-4131-8131-313131313131";
const SECOND: &str = "32323232-3232-4232-8232-323232323232";
const SESSION: &str = "registry-lock-chat";
const RUN: &str = "registry-lock-run";

fn connection(id: &str, workspace: &str, target: &str) -> Connection {
    Connection {
        id: id.into(),
        node_id: None,
        name: format!("connection {id}"),
        ssh_target: target.into(),
        port: Some(22),
        identity_file: None,
        proxy_jump: None,
        socket_path: "/tmp/registry-lock.sock".into(),
        owner_uid: 10001,
        workspace_id: workspace.into(),
        workspace_public_key: "11".repeat(32),
        remote_root: None,
        remote_execution: false,
        cluster_connection_id: format!("cluster-{id}"),
        mode: ClusterMode::Public,
        institution_id: None,
        policy_epoch: 1,
        status: "disconnected".into(),
        last_error: None,
        device_id: "22".repeat(32),
        public_key: "33".repeat(32),
    }
}

/// A live grant for [`SESSION`] on `connection_id`, recorded before grants were bound, so it
/// stands as its chat's own with no chat under the id.
fn grant(connection_id: &str, run_id: &str) -> Scope {
    Scope {
        connection_id: connection_id.into(),
        run_id: run_id.into(),
        channel_id: "registry-lock-channel".into(),
        source_channels: vec!["registry-lock-channel".into()],
        epoch: 1,
        provider_binding: "registry-lock-provider".into(),
        public_provider: true,
        origin_restricted: false,
        institution_ids: BTreeSet::new(),
        institution_policy: true,
        expired: false,
        expires_at: None,
        labels: None,
        session_incarnation: None,
        revocation: None,
    }
}

#[cfg(unix)]
fn public_call() -> CallCapability {
    CallCapability::for_test(ProviderTier::Public, true)
}

/// One directory, seeded with `registry`, and two managers loaded from it — two processes.
struct Profile {
    _data: TempDir,
    crew_root: TempDir,
    first: CrewManager,
    second: CrewManager,
}

impl Profile {
    async fn new(registry: Registry) -> Self {
        let data = TempDir::new().unwrap();
        let crew_root = TempDir::new().unwrap();
        fs::write(
            crew_root.path().join("connections.json"),
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        let store = Arc::new(SessionManager::new(data.path().to_path_buf()));
        let open = || {
            let crew = CrewManager::new(crew_root.path().to_path_buf()).unwrap();
            crew.use_session_store(store.clone());
            crew
        };
        let (first, second) = (open(), open());
        Self {
            _data: data,
            crew_root,
            first,
            second,
        }
    }

    fn saved(&self) -> Value {
        serde_json::from_slice(&fs::read(self.crew_root.path().join("connections.json")).unwrap())
            .unwrap()
    }

    fn saved_connection_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.saved()["connections"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap().to_owned())
            .collect();
        ids.sort();
        ids
    }
}

async fn connection_ids(crew: &CrewManager) -> Vec<String> {
    let mut ids: Vec<String> = crew.list().await.into_iter().map(|c| c.id).collect();
    ids.sort();
    ids
}

/// Credentials in files under the test's own profile, never the OS keychain: saving a
/// connection writes a device key, and a revoke reads one. Only in a process of its own.
#[cfg(unix)]
fn file_credentials(root: &Path) -> env_lock::EnvGuard<'static> {
    let profile = root.join("profile");
    fs::create_dir_all(&profile).unwrap();
    let profile = profile.to_string_lossy().into_owned();
    crate::test_sandbox::relocate_path_root_and(
        profile.as_str(),
        [
            ("BIOROUTER_DEV_PROFILE_ROOT", Some(profile.as_str())),
            ("BIOROUTER_DISABLE_KEYRING", Some("true")),
        ],
    )
}

#[cfg(unix)]
fn new_connection(workspace_id: &str, target: &str) -> SaveConnection {
    SaveConnection {
        preparation_id: None,
        name: "saved by the first process".into(),
        ssh_target: target.into(),
        port: Some(22),
        identity_file: None,
        proxy_jump: None,
        socket_path: "/tmp/registry-lock-saved.sock".into(),
        owner_uid: 10001,
        workspace_id: workspace_id.into(),
        workspace_public_key: "44".repeat(32),
        remote_root: None,
        remote_execution: false,
        cluster_connection_id: None,
        mode: ClusterMode::Public,
        institution_id: None,
    }
}

/// One process saves a connection, then another — loaded before that save — revokes a grant.
/// Both changes land on disk, and each process holds both after its next write.
#[cfg(unix)]
#[tokio::test]
async fn a_save_and_a_revoke_from_two_processes_both_survive() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let scratch = TempDir::new().unwrap();
    let _env = file_credentials(scratch.path());
    let profile = Profile::new(Registry {
        connections: vec![connection(
            FIRST,
            "41414141-4141-4141-8141-414141414141",
            "crew@first.invalid",
        )],
        scopes: HashMap::from([(SESSION.into(), grant(FIRST, RUN))]),
        ..Registry::default()
    })
    .await;

    let saved = profile
        .first
        .save(new_connection(
            "42424242-4242-4242-8242-424242424242",
            "crew@saved.invalid",
        ))
        .await
        .unwrap();
    let outcome = profile
        .second
        .revoke_session_if_current(SESSION, RUN)
        .await
        .expect("the stop is saved even though no workspace answers");
    assert!(!outcome.remote_confirmed);

    // On disk: the connection the first saved, and the grant the second revoked.
    let mut expected = vec![FIRST.to_owned(), saved.id.clone()];
    expected.sort();
    assert_eq!(
        profile.saved_connection_ids(),
        expected,
        "the revoke wrote the first process's new connection away"
    );
    assert_eq!(profile.saved()["scopes"][SESSION]["expired"], json!(true));

    // The second took the first's connection in with its own write.
    assert_eq!(connection_ids(&profile.second).await, expected);

    // The first takes the revocation in with its next write, whatever that write is.
    profile.first.prepare_device().await.unwrap();
    assert!(profile.first.registry.lock().await.scopes[SESSION].expired);
    assert_eq!(
        profile
            .first
            .check_dispatch(SESSION, &public_call())
            .await
            .unwrap_err()
            .to_string(),
        GRANT_REVOKED
    );
    assert_eq!(profile.saved()["scopes"][SESSION]["expired"], json!(true));
    assert_eq!(profile.saved_connection_ids(), expected);
    assert!(profile.saved()["pending_device"].is_object());
}

/// A grant one process revoked is never written back as live by another that loaded it
/// live — through any of the writes that used to save a whole stale copy.
#[cfg(unix)]
#[tokio::test]
async fn a_revoke_is_never_written_back_live_by_another_process() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let scratch = TempDir::new().unwrap();
    let _env = file_credentials(scratch.path());
    let other_session = "registry-lock-other-chat";
    let profile = Profile::new(Registry {
        connections: vec![
            connection(
                FIRST,
                "41414141-4141-4141-8141-414141414141",
                "crew@first.invalid",
            ),
            connection(
                SECOND,
                "43434343-4343-4343-8343-434343434343",
                "crew@second.invalid",
            ),
        ],
        scopes: HashMap::from([
            (SESSION.into(), grant(FIRST, RUN)),
            (
                other_session.into(),
                grant(FIRST, "registry-lock-other-run"),
            ),
        ]),
        ..Registry::default()
    })
    .await;
    // Both processes have looked at the grant, live.
    for crew in [&profile.first, &profile.second] {
        crew.check_dispatch(SESSION, &public_call()).await.unwrap();
    }

    profile
        .second
        .revoke_session_if_current(SESSION, RUN)
        .await
        .unwrap();
    assert_eq!(profile.saved()["scopes"][SESSION]["expired"], json!(true));

    let still_revoked = |step: &str| {
        let saved = profile.saved();
        assert_eq!(
            saved["scopes"][SESSION]["expired"],
            json!(true),
            "{step} wrote a revoked grant back as live"
        );
        assert_eq!(saved["scopes"][SESSION]["run_id"], json!(RUN), "{step}");
    };

    // Every kind of write the first process makes, each of which once saved its whole
    // (stale, live) copy.
    profile.first.remove(SECOND).await.unwrap();
    still_revoked("removing another connection");
    profile.first.prepare_device().await.unwrap();
    still_revoked("preparing a device");
    profile
        .first
        .save(new_connection(
            "42424242-4242-4242-8242-424242424242",
            "crew@saved.invalid",
        ))
        .await
        .unwrap();
    still_revoked("saving a connection");
    profile
        .first
        .revoke_session_if_current(other_session, "registry-lock-other-run")
        .await
        .unwrap();
    still_revoked("revoking another grant");
    profile
        .first
        .retire_deleted_sessions(&[(other_session.into(), 1)], Path::new("/elsewhere"))
        .await
        .unwrap();
    still_revoked("retiring a deleted chat's grant");

    // And the first process itself now refuses it.
    assert!(profile.first.registry.lock().await.scopes[SESSION].expired);
    assert_eq!(
        profile
            .first
            .check_dispatch(SESSION, &public_call())
            .await
            .unwrap_err()
            .to_string(),
        GRANT_REVOKED
    );
    // Its own revoke landed too, and the connection it removed stays removed.
    let saved = profile.saved();
    assert_eq!(saved["scopes"][other_session]["expired"], json!(true));
    assert!(!profile.saved_connection_ids().contains(&SECOND.to_owned()));
}

/// A stop this process could not save still holds here when another process's write is read
/// back: re-reading the file never revives a grant this process revoked.
#[tokio::test]
async fn a_stop_that_could_not_be_saved_survives_another_process_write() {
    let profile = Profile::new(Registry {
        connections: vec![connection(
            FIRST,
            "41414141-4141-4141-8141-414141414141",
            "crew@first.invalid",
        )],
        scopes: HashMap::from([(SESSION.into(), grant(FIRST, RUN))]),
        ..Registry::default()
    })
    .await;
    // This process stopped the grant but could not save the stop.
    profile
        .first
        .registry
        .lock()
        .await
        .scopes
        .get_mut(SESSION)
        .unwrap()
        .expired = true;
    // Another process writes meanwhile, with the grant still live on disk.
    profile
        .second
        .update_registry(|r| {
            r.connections.push(connection(
                SECOND,
                "43434343-4343-4343-8343-434343434343",
                "crew@second.invalid",
            ));
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(profile.saved()["scopes"][SESSION]["expired"], json!(false));

    // This process's next write reads the other's back — and keeps its stop, and saves it.
    profile.first.update_registry(|_| Ok(())).await.unwrap();
    assert!(profile.first.registry.lock().await.scopes[SESSION].expired);
    assert_eq!(profile.saved()["scopes"][SESSION]["expired"], json!(true));
    assert_eq!(
        connection_ids(&profile.first).await,
        vec![FIRST.to_owned(), SECOND.to_owned()]
    );
}

/// What the file never holds for this process is carried over when another's write is read
/// back: this process's live connection status, and a binding it made in memory. A
/// connection only the other process knows reads as a fresh load reads it.
#[tokio::test]
async fn reading_another_process_write_keeps_this_process_state() {
    let profile = Profile::new(Registry {
        connections: vec![connection(
            FIRST,
            "41414141-4141-4141-8141-414141414141",
            "crew@first.invalid",
        )],
        scopes: HashMap::from([(SESSION.into(), grant(FIRST, RUN))]),
        ..Registry::default()
    })
    .await;
    {
        let mut here = profile.first.registry.lock().await;
        here.connections[0].status = "connected".into();
        here.scopes.get_mut(SESSION).unwrap().session_incarnation = Some(42);
    }
    profile
        .second
        .update_registry(|r| {
            let mut other = connection(
                SECOND,
                "43434343-4343-4343-8343-434343434343",
                "crew@second.invalid",
            );
            other.status = "connected".into();
            other.last_error = Some("the other process's".into());
            r.connections.push(other);
            Ok(())
        })
        .await
        .unwrap();

    profile.first.update_registry(|_| Ok(())).await.unwrap();
    let here = profile.first.registry.lock().await;
    let status = |id: &str| {
        let c = here.connections.iter().find(|c| c.id == id).unwrap();
        (c.status.clone(), c.last_error.clone())
    };
    assert_eq!(status(FIRST), ("connected".into(), None));
    assert_eq!(status(SECOND), ("disconnected".into(), None));
    assert_eq!(here.scopes[SESSION].session_incarnation, Some(42));
}

/// A grant another process replaced (a new run under the same chat) is the file's: this
/// process's stale copy of the old one, expired or not, never overrides it.
#[tokio::test]
async fn a_newer_grant_from_another_process_is_not_overridden() {
    let profile = Profile::new(Registry {
        connections: vec![connection(
            FIRST,
            "41414141-4141-4141-8141-414141414141",
            "crew@first.invalid",
        )],
        scopes: HashMap::from([(SESSION.into(), grant(FIRST, RUN))]),
        ..Registry::default()
    })
    .await;
    profile
        .first
        .registry
        .lock()
        .await
        .scopes
        .get_mut(SESSION)
        .unwrap()
        .expired = true;
    profile
        .second
        .update_registry(|r| {
            r.scopes
                .insert(SESSION.into(), grant(FIRST, "registry-lock-newer-run"));
            Ok(())
        })
        .await
        .unwrap();

    profile.first.update_registry(|_| Ok(())).await.unwrap();
    let scope = profile.first.registry.lock().await.scopes[SESSION].clone();
    assert_eq!(scope.run_id, "registry-lock-newer-run");
    assert!(
        !scope.expired,
        "an old run's stop leaked onto a newer grant"
    );
    assert_eq!(profile.saved()["scopes"][SESSION]["expired"], json!(false));
}

/// Two processes writing at once lose nothing: every update runs against the file as the
/// last one left it, under the shared lock.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_writes_from_two_processes_all_land() {
    let profile = Arc::new(Profile::new(Registry::default()).await);
    let mut writes = Vec::new();
    for index in 0..24 {
        let profile = profile.clone();
        writes.push(tokio::spawn(async move {
            let crew = if index % 2 == 0 {
                &profile.first
            } else {
                &profile.second
            };
            crew.update_registry(|r| {
                r.connections.push(connection(
                    &format!("concurrent-{index:02}"),
                    "41414141-4141-4141-8141-414141414141",
                    "crew@concurrent.invalid",
                ));
                Ok(())
            })
            .await
            .unwrap();
        }));
    }
    for write in writes {
        write.await.unwrap();
    }
    let expected: Vec<String> = (0..24)
        .map(|index| format!("concurrent-{index:02}"))
        .collect();
    assert_eq!(profile.saved_connection_ids(), expected);
    for crew in [&profile.first, &profile.second] {
        crew.update_registry(|_| Ok(())).await.unwrap();
        assert_eq!(connection_ids(crew).await, expected);
    }
}

/// While another process holds the lock, an update waits for it; one held past the bound is
/// refused plainly, and changes nothing anywhere.
#[tokio::test(start_paused = true)]
async fn an_update_waits_for_the_lock_and_gives_up_plainly() {
    let profile = Profile::new(Registry {
        connections: vec![connection(
            FIRST,
            "41414141-4141-4141-8141-414141414141",
            "crew@first.invalid",
        )],
        ..Registry::default()
    })
    .await;
    let before = fs::read(profile.crew_root.path().join("connections.json")).unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(profile.crew_root.path().join("connections.lock"))
        .unwrap();
    held.lock().unwrap();

    let refused = profile
        .first
        .update_registry(|r| {
            r.connections.clear();
            Ok(())
        })
        .await
        .unwrap_err();
    assert_eq!(refused.to_string(), REGISTRY_BUSY);
    assert_eq!(
        fs::read(profile.crew_root.path().join("connections.json")).unwrap(),
        before
    );
    assert_eq!(connection_ids(&profile.first).await, vec![FIRST.to_owned()]);

    // Released while one waits: it proceeds.
    let first = &profile.first;
    let waiting = first.update_registry(|r| {
        r.connections.clear();
        Ok(())
    });
    let release = async {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        held.unlock().unwrap();
    };
    let (updated, ()) = tokio::join!(waiting, release);
    updated.unwrap();
    assert!(connection_ids(&profile.first).await.is_empty());
    assert_eq!(profile.saved_connection_ids(), Vec::<String>::new());
}

/// Deleting a chat in a profile that never used Crew creates nothing: every chat delete
/// reaches the registry, and it must not make a Crew directory to find nothing in it.
#[tokio::test]
async fn retiring_in_a_profile_without_crew_creates_nothing() {
    let parent = TempDir::new().unwrap();
    let root = parent.path().join("crew");
    let crew = CrewManager::new(root.clone()).unwrap();
    crew.retire_deleted_sessions(&[("some-chat".into(), 7)], parent.path())
        .await
        .unwrap();
    assert!(!root.exists());
}

/// A refused edit changes nothing, in memory or on disk, even when another process wrote in
/// between — and an edit that changes nothing does not rewrite the file.
#[tokio::test]
async fn a_refused_or_empty_edit_writes_nothing() {
    let profile = Profile::new(Registry {
        connections: vec![connection(
            FIRST,
            "41414141-4141-4141-8141-414141414141",
            "crew@first.invalid",
        )],
        ..Registry::default()
    })
    .await;
    let path = profile.crew_root.path().join("connections.json");
    let before = fs::read(&path).unwrap();

    let refused = profile
        .first
        .update_registry(|r| -> Result<()> {
            r.connections.clear();
            anyhow::bail!("refused")
        })
        .await
        .unwrap_err();
    assert_eq!(refused.to_string(), "refused");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(connection_ids(&profile.first).await, vec![FIRST.to_owned()]);

    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    profile.first.update_registry(|_| Ok(())).await.unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
