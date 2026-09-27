//! CROSSCUT-1, DAEMON-6 and CROSSCUT-8: what one process asks the saved Crew registry.
//!
//! ⚠ **Grant state; a change here needs human review.** Each test opens managers on one
//! directory the way separate processes do, each with its own copy and its own session store
//! handle, and checks the answers every Crew boundary reads: whether a chat is scoped, which
//! chats listings leave out, which tools a chat may call, and whether its model may be used.

use super::*;
use crate::{
    privacy::{CallCapability, ProviderTier},
    session::{session_manager::SessionType, SessionManager},
};
use std::fs;
use tempfile::TempDir;

const CONNECTION: &str = "freshness-connection";
const RUN: &str = "freshness-run";

fn connection() -> Connection {
    Connection {
        id: CONNECTION.into(),
        node_id: None,
        name: "methods".into(),
        ssh_target: "crew@crew.invalid".into(),
        port: Some(22),
        identity_file: None,
        proxy_jump: None,
        socket_path: "/tmp/freshness.sock".into(),
        owner_uid: 10001,
        workspace_id: "workspace-methods".into(),
        workspace_public_key: "11".repeat(32),
        remote_root: None,
        remote_execution: false,
        cluster_connection_id: "cluster-methods".into(),
        mode: ClusterMode::Public,
        institution_id: None,
        policy_epoch: 1,
        status: "disconnected".into(),
        last_error: None,
        device_id: "22".repeat(32),
        public_key: "33".repeat(32),
    }
}

fn grant(incarnation: i64, expires_at: Option<u64>) -> Scope {
    Scope {
        connection_id: CONNECTION.into(),
        run_id: RUN.into(),
        channel_id: "channel-methods".into(),
        source_channels: vec!["channel-methods".into()],
        epoch: 1,
        provider_binding: "freshness-provider".into(),
        public_provider: true,
        origin_restricted: false,
        institution_ids: BTreeSet::new(),
        institution_policy: true,
        expired: false,
        expires_at,
        labels: None,
        session_incarnation: Some(incarnation),
        revocation: None,
    }
}

fn public_call() -> CallCapability {
    CallCapability::for_test(ProviderTier::Public, true)
}

/// One profile's session store and Crew directory, opened by as many processes as a test needs.
struct Profile {
    data: TempDir,
    crew_root: TempDir,
    store: Arc<SessionManager>,
}

impl Profile {
    fn new() -> Self {
        let data = TempDir::new().unwrap();
        let store = Arc::new(SessionManager::new(data.path().to_path_buf()));
        Self {
            data,
            crew_root: TempDir::new().unwrap(),
            store,
        }
    }

    /// A process opening the profile now.
    fn open(&self) -> CrewManager {
        let crew = CrewManager::new(self.crew_root.path().to_path_buf()).unwrap();
        crew.use_session_store(self.store.clone());
        crew
    }

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

    fn registry_path(&self) -> PathBuf {
        self.crew_root.path().join("connections.json")
    }

    fn write(&self, registry: &Value) {
        fs::write(self.registry_path(), serde_json::to_vec(registry).unwrap()).unwrap();
    }
}

/// CROSSCUT-1: a terminal chat opened before the desktop granted another chat learns of the
/// grant at its next question. It used to answer from the copy it loaded first, so the Crew
/// chat was listed, readable and searchable from the terminal, and its tools unrestricted.
#[tokio::test]
async fn a_grant_another_process_makes_restricts_the_chat_here_at_once() {
    let profile = Profile::new();
    let (crew_chat, incarnation) = profile.chat().await;
    let (other_chat, _) = profile.chat().await;

    // The terminal's process loads the registry before the grant exists.
    let terminal = profile.open();
    assert!(!terminal.is_scoped_session(&crew_chat).await);
    assert!(terminal.scoped_session_ids().await.is_empty());

    // The desktop's daemon, another process, grants the chat.
    let daemon = profile.open();
    daemon
        .update_registry(|r| {
            r.connections.push(connection());
            r.scopes.insert(crew_chat.clone(), grant(incarnation, None));
            Ok(())
        })
        .await
        .unwrap();

    assert!(terminal.is_scoped_session(&crew_chat).await);
    assert_eq!(
        terminal.scoped_session_ids().await,
        HashSet::from([crew_chat.clone()])
    );
    assert!(
        terminal
            .authorize_session_tool(&crew_chat, "developer__shell")
            .await
            .is_err(),
        "the Crew chat's shell is withheld here too"
    );
    terminal
        .authorize_session_tool(&crew_chat, "crew__request")
        .await
        .unwrap();
    terminal
        .check_dispatch(&crew_chat, &public_call())
        .await
        .unwrap();
    // Nothing changes for any other chat.
    assert!(!terminal.is_scoped_session(&other_chat).await);
    terminal
        .authorize_session_tool(&other_chat, "developer__shell")
        .await
        .unwrap();

    // The daemon revokes it; the terminal refuses it at its next question.
    daemon
        .update_registry(|r| {
            r.scopes.get_mut(&crew_chat).unwrap().expired = true;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        terminal
            .check_dispatch(&crew_chat, &public_call())
            .await
            .unwrap_err()
            .to_string(),
        GRANT_REVOKED
    );
    assert!(terminal.is_scoped_session(&crew_chat).await);
}

/// A file read again carries this process's own state in, as a save does: a stop this process
/// made and could not save is never brought back to life by another process's copy.
#[tokio::test]
async fn reading_the_file_again_keeps_a_stop_made_here() {
    let profile = Profile::new();
    let (chat, incarnation) = profile.chat().await;
    let daemon = profile.open();
    daemon
        .update_registry(|r| {
            r.connections.push(connection());
            r.scopes.insert(chat.clone(), grant(incarnation, None));
            Ok(())
        })
        .await
        .unwrap();
    let terminal = profile.open();
    terminal
        .check_dispatch(&chat, &public_call())
        .await
        .unwrap();
    // Stopped here, in memory only (as when the save failed).
    terminal
        .registry
        .lock()
        .await
        .scopes
        .get_mut(&chat)
        .unwrap()
        .expired = true;
    // The other process writes the grant, still live in its copy, back with another change.
    daemon
        .update_registry(|r| {
            r.connections[0].name = "methods, renamed".into();
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        terminal
            .check_dispatch(&chat, &public_call())
            .await
            .unwrap_err()
            .to_string(),
        GRANT_REVOKED
    );
    assert_eq!(
        terminal.connection(CONNECTION).await.unwrap().name,
        "methods, renamed",
        "the rest of the file was read"
    );
}

/// DAEMON-6: a registry this build cannot read (here a revocation state a newer build wrote)
/// restricts the chats it names and nothing else. It used to fail the Crew manager itself, and
/// with it every chat's model and tool calls. Nothing is saved over it, and a repaired file is
/// read again.
#[tokio::test]
async fn an_unreadable_registry_restricts_only_the_chats_it_names() {
    let profile = Profile::new();
    let (crew_chat, incarnation) = profile.chat().await;
    let (other_chat, _) = profile.chat().await;
    let mut scope = serde_json::to_value(grant(incarnation, None)).unwrap();
    scope["expired"] = json!(true);
    scope["revocation"] = json!("future_value");
    let unreadable = json!({
        "connections": [serde_json::to_value(connection()).unwrap()],
        "scopes": {crew_chat.clone(): scope},
    });
    profile.write(&unreadable);
    let bytes = fs::read(profile.registry_path()).unwrap();

    let crew = profile.open();
    assert!(crew.is_scoped_session(&crew_chat).await);
    assert_eq!(
        crew.check_dispatch(&crew_chat, &public_call())
            .await
            .unwrap_err()
            .to_string(),
        freshness::REGISTRY_UNREADABLE
    );
    assert!(crew
        .authorize_session_tool(&crew_chat, "developer__shell")
        .await
        .is_err());
    assert!(crew.run_metadata(&crew_chat).await.is_none());
    assert_eq!(
        crew.scoped_session_ids().await,
        HashSet::from([crew_chat.clone()])
    );

    // An ordinary chat works as it always did.
    assert!(!crew.is_scoped_session(&other_chat).await);
    crew.check_dispatch(&other_chat, &public_call())
        .await
        .unwrap();
    crew.authorize_session_tool(&other_chat, "developer__shell")
        .await
        .unwrap();

    // Nothing is saved over what could not be read.
    let refused = crew
        .update_registry(|r| {
            r.completed_preparations.insert("x".into(), "y".into());
            Ok(())
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("can't be read"), "{refused}");
    assert_eq!(fs::read(profile.registry_path()).unwrap(), bytes);

    // Repaired (the value this build knows), it is read again at the next question.
    let mut repaired = unreadable.clone();
    repaired["scopes"][&crew_chat]["revocation"] = json!("confirmed");
    profile.write(&repaired);
    assert_eq!(
        crew.check_dispatch(&crew_chat, &public_call())
            .await
            .unwrap_err()
            .to_string(),
        GRANT_REVOKED
    );
    crew.update_registry(|_| Ok(())).await.unwrap();
}

/// A registry that names no chat this build can read may restrict any of them, so every chat
/// is restricted until it is repaired, and listings leave every chat out.
#[tokio::test]
async fn a_registry_that_names_no_readable_chat_restricts_every_chat() {
    let profile = Profile::new();
    let (chat, _) = profile.chat().await;
    // A chat with something in it: an empty one holds no context to leave out.
    profile
        .store
        .add_message(
            &chat,
            &crate::conversation::message::Message::user().with_text("the methods thread"),
        )
        .await
        .unwrap();
    fs::write(profile.registry_path(), b"{\"connections\": [").unwrap();
    let crew = profile.open();
    assert!(crew.is_scoped_session(&chat).await);
    assert!(crew.is_scoped_session("any-other-chat").await);
    assert!(crew.scoped_session_ids().await.contains(&chat));
    assert_eq!(
        crew.check_dispatch(&chat, &public_call())
            .await
            .unwrap_err()
            .to_string(),
        freshness::REGISTRY_UNREADABLE
    );

    // Repaired, nothing is restricted that holds no grant.
    fs::write(
        profile.registry_path(),
        b"{\"connections\": [], \"scopes\": {}}",
    )
    .unwrap();
    assert!(!crew.is_scoped_session(&chat).await);
    assert!(crew.scoped_session_ids().await.is_empty());
}

/// What an unreadable file names is read without trusting any of it: a chat's grant under
/// `scopes` or an earlier grant's `session_id`, and nothing at all when either cannot be read.
#[test]
fn an_unreadable_registry_names_its_chats_or_none() {
    let named = |text: &str| freshness::Unreadable::of(text.as_bytes()).sessions;
    assert_eq!(
        named(r#"{"scopes": {"a": 1}, "replaced": [{"session_id": "b"}]}"#),
        Some(HashSet::from(["a".to_owned(), "b".to_owned()]))
    );
    assert_eq!(named(r#"{"scopes": {}}"#), Some(HashSet::new()));
    for unnamed in [
        "{",
        "[]",
        r#"{"connections": []}"#,
        r#"{"scopes": []}"#,
        r#"{"scopes": {}, "replaced": {}}"#,
        r#"{"scopes": {}, "replaced": [{"scope": {}}]}"#,
    ] {
        assert_eq!(named(unnamed), None, "{unnamed}");
    }
}

/// CROSSCUT-8: a deleted chat's grant is forgotten once it is settled, a week past its run's
/// end, with its run credential. Session ids are single use, so the pruning that waited for a
/// later chat under the id never came: every deleted chat's grant stayed listed, and cost a
/// database query on every scope question, for good. A grant not yet settled, or whose end was
/// never recorded, still restricts and stays listed.
#[tokio::test]
async fn a_deleted_chats_grant_is_forgotten_once_settled() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let scratch = TempDir::new().unwrap();
    let profile_dir = scratch.path().join("profile");
    fs::create_dir_all(&profile_dir).unwrap();
    let profile_dir = profile_dir.to_string_lossy().into_owned();
    let _env = crate::test_sandbox::relocate_path_root_and(
        profile_dir.as_str(),
        [
            ("BIOROUTER_DEV_PROFILE_ROOT", Some(profile_dir.as_str())),
            ("BIOROUTER_DISABLE_KEYRING", Some("true")),
        ],
    );
    let profile = Profile::new();
    let crew = profile.open();
    let now = revocation::unix_now();
    let week = revocation::REPLACED_KEPT_PAST_END;
    let mut chats = Vec::new();
    for end in [Some(now - week - 60), Some(now - 60), None] {
        let (chat, incarnation) = profile.chat().await;
        crew.update_registry(|r| {
            if r.connections.is_empty() {
                r.connections.push(connection());
            }
            r.scopes.insert(chat.clone(), grant(incarnation, end));
            Ok(())
        })
        .await
        .unwrap();
        crew.write_credential(&format!("run:{chat}"), "run-credential")
            .unwrap();
        profile.store.delete_session(&chat).await.unwrap();
        chats.push(chat);
    }
    let [settled, recent, unknown] = [&chats[0], &chats[1], &chats[2]];

    assert!(!crew.is_scoped_session(settled).await);
    assert!(!crew.registry.lock().await.scopes.contains_key(settled));
    let saved: Value = serde_json::from_slice(&fs::read(profile.registry_path()).unwrap()).unwrap();
    assert!(saved["scopes"].get(settled).is_none());
    assert!(crew.read_credential(&format!("run:{settled}")).is_err());

    for kept in [recent, unknown] {
        assert!(crew.is_scoped_session(kept).await, "{kept}");
        assert_eq!(
            crew.check_dispatch(kept, &public_call())
                .await
                .unwrap_err()
                .to_string(),
            GRANT_GONE
        );
        assert!(crew.read_credential(&format!("run:{kept}")).is_ok());
    }
    let listed = crew.session_grants(CONNECTION).await.unwrap();
    let listed: HashSet<&str> = listed["grants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["session_id"].as_str().unwrap())
        .collect();
    assert_eq!(listed, HashSet::from([recent.as_str(), unknown.as_str()]));
}
