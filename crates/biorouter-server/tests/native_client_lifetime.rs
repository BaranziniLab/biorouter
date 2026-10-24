//! Real transport/auth/lifetime controls with synthetic files; no model or clinical data.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::{path::Path, time::Duration};

use biorouter::{
    conversation::message::{ActionRequiredData, MessageContent},
    folder_access::{FileAdmission, FolderAccessManager},
    native_client::{self, NativeClient},
    pending_user_action::{
        DecisionAuthority, PendingUserActions, ResolveOutcome, UserActionOutcome,
    },
    permission::Permission,
};
use biorouter_mcp::{FileAccessPlan, FileAccessRequest};
use biorouter_sandbox::{bounded_fs::authorized_path, AccessMode};
use futures::StreamExt;
use sha2::Digest;
use tokio_tungstenite::{
    tungstenite::{client::IntoClientRequest, Error as WsError},
    WebSocketStream,
};
use tokio_util::sync::CancellationToken;

#[cfg(unix)]
use std::path::PathBuf;

const SECRET: &str = "synthetic-native-api-secret";
const PROOF: &str = "synthetic-native-user-proof";
const INSTANCE: &str = "66666666-6666-4666-8666-666666666666";

#[cfg(unix)]
type Transport = tokio::net::UnixStream;
#[cfg(not(unix))]
type Transport = tokio::net::TcpStream;
#[cfg(unix)]
type Endpoint = PathBuf;
#[cfg(not(unix))]
type Endpoint = std::net::SocketAddr;

async fn connect(
    endpoint: &Endpoint,
    proof: Option<&str>,
    secret: &str,
    owner: Option<&str>,
) -> Result<WebSocketStream<Transport>, WsError> {
    connect_instance(endpoint, proof, secret, owner, Some(INSTANCE)).await
}

async fn connect_instance(
    endpoint: &Endpoint,
    proof: Option<&str>,
    secret: &str,
    owner: Option<&str>,
    instance: Option<&str>,
) -> Result<WebSocketStream<Transport>, WsError> {
    let mut request = "ws://localhost/daemon/native-client"
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("x-secret-key", secret.parse().unwrap());
    if let Some(instance) = instance {
        request
            .headers_mut()
            .insert("x-daemon-instance", instance.parse().unwrap());
    }
    if let Some(owner) = owner {
        request
            .headers_mut()
            .insert("x-biorouter-native-client", owner.parse().unwrap());
    }
    if let Some(proof) = proof {
        request
            .headers_mut()
            .insert("x-user-action", proof.parse().unwrap());
    }
    #[cfg(unix)]
    let stream = Transport::connect(endpoint).await.unwrap();
    #[cfg(not(unix))]
    let stream = Transport::connect(endpoint).await.unwrap();
    tokio_tungstenite::client_async(request, stream)
        .await
        .map(|(socket, _)| socket)
}

async fn wait_until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

async fn registered(endpoint: &Endpoint) -> (WebSocketStream<Transport>, NativeClient) {
    let mut socket = connect(endpoint, Some(PROOF), SECRET, None).await.unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let client = NativeClient::lookup(frame.to_text().unwrap()).unwrap();
    (socket, client)
}

async fn approve(
    session: &str,
    workspace: &Path,
    file: &Path,
    owner: Option<NativeClient>,
) -> FileAdmission {
    let session_owned = session.to_string();
    let workspace = workspace.to_path_buf();
    let file = file.to_path_buf();
    let answering = owner.clone();
    let task = tokio::spawn(native_client::scope(owner, async move {
        FolderAccessManager::global()
            .admit(
                &session_owned,
                "developer__text_editor",
                &workspace,
                &FileAccessPlan {
                    requests: vec![FileAccessRequest {
                        path: file,
                        access: AccessMode::Read,
                        recursive: false,
                    }],
                },
                &CancellationToken::new(),
            )
            .await
    }));
    let id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            for message in PendingUserActions::global().pending_cards_for_session(session) {
                for content in message.content {
                    if let MessageContent::ActionRequired(action) = content {
                        if let ActionRequiredData::ToolConfirmation { id, .. } = action.data {
                            return id;
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let authority = answering
        .as_ref()
        .map(DecisionAuthority::from_native_folder_access_proof)
        .unwrap_or_else(DecisionAuthority::from_local_human_surface);
    assert_eq!(
        PendingUserActions::global().resolve_in_session(
            session,
            &id,
            UserActionOutcome::Approved {
                permission: Permission::AlwaysAllow
            },
            authority
        ),
        ResolveOutcome::Delivered
    );
    task.await.unwrap().unwrap()
}

async fn mounted_server(root: &Path) -> (Endpoint, tokio::task::JoinHandle<()>) {
    biorouter_server::auth::install_user_action_digest(Some(
        sha2::Sha256::digest(PROOF.as_bytes()).into(),
    ));
    biorouter_server::daemon_service::install(biorouter::daemon_runtime::Identity {
        version: 1,
        profile_id: uuid::Uuid::new_v4().to_string(),
        instance_id: INSTANCE.to_string(),
        pid: std::process::id(),
        user_action_installed: true,
    })
    .unwrap();
    let state = biorouter_server::state::AppState::new().await.unwrap();
    let app = biorouter_server::routes::configure(state, SECRET.to_string()).layer(
        axum::middleware::from_fn_with_state(
            SECRET.to_string(),
            biorouter_server::auth::check_token,
        ),
    );
    #[cfg(unix)]
    let endpoint = root.join("native.sock");
    #[cfg(unix)]
    let listener = tokio::net::UnixListener::bind(&endpoint).unwrap();
    #[cfg(not(unix))]
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    #[cfg(not(unix))]
    let endpoint = listener.local_addr().unwrap();
    let server = tokio::spawn(async {
        axum::serve(listener, app).await.unwrap();
    });
    (endpoint, server)
}

async fn repeat_retired_owners(
    endpoint: &Endpoint,
    session: &str,
    workspace: &Path,
    sentinel: &Path,
) {
    let manager = FolderAccessManager::global();
    // More than the global per-chat grant cap must remain usable after dead owners are pruned.
    for _ in 0..130 {
        let (mut socket, owner) = registered(endpoint).await;
        let admission = approve(session, workspace, sentinel, Some(owner.clone())).await;
        assert_eq!(
            native_client::scope(Some(owner.clone()), async {
                manager.list_session(session).len()
            })
            .await,
            1
        );
        socket.close(None).await.unwrap();
        wait_until(|| NativeClient::lookup(&owner.id().to_string()).is_none()).await;
        drop(admission);
    }
}

async fn assert_registration_refusals(endpoint: &Endpoint) {
    for (proof, secret, expected) in [
        (None, SECRET, 403),
        (Some("forged-proof"), SECRET, 403),
        (Some(PROOF), "wrong-api-secret", 401),
    ] {
        let error = connect(endpoint, proof, secret, None).await.unwrap_err();
        let WsError::Http(response) = error else {
            panic!("expected authentication refusal")
        };
        assert_eq!(response.status().as_u16(), expected);
    }
    for instance in [None, Some("77777777-7777-4777-8777-777777777777")] {
        let error = connect_instance(endpoint, Some(PROOF), SECRET, None, instance)
            .await
            .unwrap_err();
        let WsError::Http(response) = error else {
            panic!("expected exact daemon-instance refusal")
        };
        assert_eq!(response.status().as_u16(), 403);
    }
}

#[tokio::test]
async fn mounted_native_socket_auth_disconnect_isolation_and_global_grant_pruning() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().canonicalize().unwrap();
    let workspace = root.join("workspace");
    let external = root.join("outside");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&external).unwrap();
    let sentinel = external.join("sentinel.txt");
    std::fs::write(&sentinel, "SYNTHETIC_CANARY").unwrap();
    let (endpoint, server) = mounted_server(&root).await;
    assert_registration_refusals(&endpoint).await;
    let session = uuid::Uuid::new_v4().to_string();
    let manager = FolderAccessManager::global();
    manager.pin_workspace(&session, &workspace).unwrap();
    let ownerless = approve(&session, &workspace, &sentinel, None).await;
    let (socket_a, owner_a) = registered(&endpoint).await;
    let (mut socket_b, owner_b) = registered(&endpoint).await;
    let error = connect(&endpoint, None, SECRET, Some(&owner_a.id().to_string()))
        .await
        .unwrap_err();
    let WsError::Http(response) = error else {
        panic!("expected refusal of an unproven native owner claim")
    };
    assert_eq!(response.status().as_u16(), 403);

    let admission_a = approve(&session, &workspace, &sentinel, Some(owner_a.clone())).await;
    let admission_b = approve(&session, &workspace, &sentinel, Some(owner_b.clone())).await;
    let retained_a = authorized_path(
        admission_a.authority.token(),
        &session,
        &admission_a.action_id,
        &sentinel,
        AccessMode::Read,
    )
    .unwrap();
    let retained_b = authorized_path(
        admission_b.authority.token(),
        &session,
        &admission_b.action_id,
        &sentinel,
        AccessMode::Read,
    )
    .unwrap();
    drop(socket_a);
    wait_until(|| NativeClient::lookup(&owner_a.id().to_string()).is_none()).await;
    assert!(retained_a.read_to_string().is_err());
    assert_eq!(retained_b.read_to_string().unwrap(), "SYNTHETIC_CANARY");
    assert_eq!(
        authorized_path(
            ownerless.authority.token(),
            &session,
            &ownerless.action_id,
            &sentinel,
            AccessMode::Read
        )
        .unwrap()
        .read_to_string()
        .unwrap(),
        "SYNTHETIC_CANARY"
    );
    assert_eq!(
        native_client::scope(Some(owner_b.clone()), async {
            manager.list_session(&session).len()
        })
        .await,
        1
    );
    assert_eq!(manager.list_session(&session).len(), 1);
    repeat_retired_owners(&endpoint, &session, &workspace, &sentinel).await;
    assert_eq!(
        native_client::scope(Some(owner_b), async {
            manager.list_session(&session).len()
        })
        .await,
        1
    );
    assert_eq!(manager.list_session(&session).len(), 1);
    assert_eq!(
        std::fs::read_to_string(sentinel).unwrap(),
        "SYNTHETIC_CANARY"
    );
    socket_b.close(None).await.unwrap();
    manager.end_session(&session);
    server.abort();
    let _ = server.await;
}
