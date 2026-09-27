//! PROVIDERS-1, PROVIDERS-3 and CROSSCUT-7: a Crew-scoped chat answers a streamed reply, and
//! asks the workspace to admit it a bounded number of times, not once per chunk.
//!
//! Two defects met on the first streamed chunk of a granted chat:
//!
//! - the reply loop read the provider through `Agent::provider()` once per streamed chunk, and
//!   each read sent a live `context.manifest` request over the Crew SSH bridge: a 600-chunk
//!   answer was 600 sequential round trips, and a bridge hiccup on any of them aborted a turn
//!   that was already answering;
//! - each of those reads also recomputed the grant's provider binding, which hashed the model's
//!   context window. Llama Server reads the loaded model's real window back on its first
//!   request, so the binding moved under the grant, and the turn was refused as "bound to its
//!   original resolved provider" as the first tokens arrived, and every turn after.
//!
//! This drives a real `Agent::reply` loop on a chat granted access the real way
//! (`CrewManager::grant_session`), on a private model that streams forty chunks and reports a
//! smaller context window once it has served its first request, as Llama Server does. Two turns
//! must both answer, and the broker must see a handful of `context.manifest` requests: one per
//! model request plus the turn-start check, never one per chunk.
//!
//! ⚠ **Its own binary**, because the Crew manager and the session store are process-global
//! and are pointed at a sandbox before `main`. Offline: the "SSH connection" is this test
//! binary itself, started by the fake `ssh` in [`FAKE_BROKER`] mode, so nothing opens a
//! socket and CI's loopback-only integration step runs it.
#![cfg(unix)]

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use biorouter::agents::extension::ExtensionConfig;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::paths::Paths;
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::privacy::affiliation::{InstitutionId, ModelAffiliation};
use biorouter::privacy::ProviderTier;
use biorouter::providers::base::{MessageStream, Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use ed25519_dalek::{Signer, SigningKey};
use futures::StreamExt;
use rmcp::model::Tool;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Set on the fake `ssh`'s child: this process is the broker, and logs every frame here.
const FAKE_BROKER: &str = "CREW_PROVIDER_ADMISSION_FAKE_BROKER";

const CONNECTION: &str = "crew-admission-connection";
const WORKSPACE: &str = "5b5b5b5b-5b5b-45b5-85b5-5b5b5b5b5b5b";
const CLUSTER: &str = "6c6c6c6c-6c6c-46c6-86c6-6c6c6c6c6c6d";
const NODE: &str = "9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f9f";
const OWNER_UID: u32 = 10001;
const CHANNEL: &str = "channel-methods";
const RUN: &str = "run-admission";
const INSTITUTION: &str = "ucsf";

/// How many chunks each streamed answer arrives in.
const CHUNKS: usize = 40;
/// The window the model table gives the model, and the smaller one the server really loaded.
const TABLE_WINDOW: usize = 262_144;
const LOADED_WINDOW: usize = 131_072;

fn workspace_key() -> SigningKey {
    SigningKey::from_bytes(&[51; 32])
}

fn device_key() -> SigningKey {
    SigningKey::from_bytes(&[52; 32])
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Where this binary's sandbox lives, set before `main`.
static SANDBOX: OnceLock<PathBuf> = OnceLock::new();

/// Before `main`: either be the fake broker (when the fake `ssh` started this binary), or
/// point every process-global store at a sandbox and put the fake `ssh` first on `PATH`.
#[ctor::ctor]
fn broker_or_sandbox() {
    if let Some(log) = std::env::var_os(FAKE_BROKER) {
        serve_as_fake_broker(Path::new(&log));
    }
    let root = tempfile::TempDir::new().expect("a sandbox root").keep();
    if Paths::path_root_override().is_none() {
        std::env::set_var("BIOROUTER_PATH_ROOT", root.join("biorouter"));
    }
    // Crew's development credential backend: device and run keys are files, never the OS
    // keychain.
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    std::env::set_var("BIOROUTER_DEV_PROFILE_ROOT", root.join("profile"));
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = std::env::current_exe().expect("this test binary");
    let log = root.join("broker-frames.log");
    for text in [exe.to_string_lossy(), log.to_string_lossy()] {
        assert!(
            !text.contains('\''),
            "a path the fake ssh cannot quote: {text}"
        );
    }
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "-G" ]; then
  printf '%s\n' 'hostname 127.0.0.1' 'port 22' 'stricthostkeychecking yes' \
    'forwardagent no' 'forwardx11 no' 'permitlocalcommand no' \
    'clearallforwardings yes' 'nohostauthenticationforlocalhost no' \
    'tunnel no' 'forkafterauthentication no' \
    'gssapidelegatecredentials no' 'proxycommand none' \
    'controlmaster no' 'controlpersist no' 'controlpath none'
  exit 0
fi
for arg; do
  if [ "$arg" = "-O" ]; then exit 0; fi
done
exec env {FAKE_BROKER}='{log}' '{exe}'
"#,
        log = log.display(),
        exe = exe.display(),
    );
    let ssh = bin.join("ssh");
    std::fs::write(&ssh, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{path}", bin.display()));
    let _ = SANDBOX.set(root);
}

/// The broker's answers, one JSON line per request line, until the bridge closes. Every frame
/// is logged first.
fn serve_as_fake_broker(log: &Path) -> ! {
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
        {
            let _ = writeln!(file, "{line}");
        }
        let Ok(frame) = serde_json::from_str::<Value>(&line) else {
            break;
        };
        let id = frame["id"].clone();
        let method = frame["method"].as_str().unwrap_or_default();
        let answer = match method {
            "hello" => Ok(signed_hello(
                frame["params"]["challenge_nonce"]
                    .as_str()
                    .unwrap_or_default(),
            )),
            "auth.challenge" => Ok(json!({
                "workspace_id": WORKSPACE, "nonce": "fake-broker-nonce", "uid": OWNER_UID,
            })),
            "workspace.snapshot" => Ok(json!({
                "workspace": {"id": WORKSPACE, "name": "Methods Lab", "mode": "private",
                    "institution_id": INSTITUTION, "policy_epoch": 1, "host_uid": OWNER_UID},
                "principals": [{"id": "principal-iris", "username": "crew_iris",
                    "display_name": "Iris Wong", "active": true}],
                "actor_id": "principal-iris",
                "teams": [],
                "channels": [{"id": CHANNEL, "name": "methods", "classification": "restricted"}],
                "protected_channel_ids": [],
            })),
            "run.create" => Ok(json!({
                "run": {"id": RUN, "protected_context": true, "expires_at": 4_102_444_800u64},
                "credential": "fake-run-credential",
            })),
            "messages.history" | "context.manifest" => Ok(json!({
                "run_id": RUN, "policy_epoch": 1, "source_channels": [CHANNEL],
                "messages": [], "people": {}, "channel_names": {CHANNEL: "methods"},
            })),
            other => Err(format!("forbidden: the fake broker refuses {other}")),
        };
        let reply = match answer {
            Ok(result) => json!({"id": id, "result": result}),
            Err(message) => json!({"id": id, "error": {"code": "forbidden", "message": message}}),
        };
        if writeln!(out, "{reply}").and_then(|()| out.flush()).is_err() {
            break;
        }
    }
    std::process::exit(0)
}

/// `hello` over `nonce`, v1-signed by the workspace key the saved connection pins.
fn signed_hello(nonce: &str) -> Value {
    let key = workspace_key();
    let public = hex(&key.verifying_key().to_bytes());
    let signature = hex(&key
        .sign(&biorouter_crew::hello_v1_payload(
            WORKSPACE, OWNER_UID, nonce, &public, NODE,
        ))
        .to_bytes());
    json!({
        "protocol": 1, "workspace_id": WORKSPACE, "host_uid": OWNER_UID,
        "workspace_public_key": public, "challenge_nonce": nonce, "node_id": NODE,
        "capabilities": ["human_chat"], "signature": signature,
    })
}

/// How many `method` frames the fake broker was sent.
fn broker_requests(method: &str) -> usize {
    let log = SANDBOX
        .get()
        .expect("the sandbox")
        .join("broker-frames.log");
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|frame| frame["method"] == method)
        .count()
}

/// A saved, private SSH connection to the fake broker, with its device key, written before
/// the Crew manager first loads.
fn save_the_connection() {
    let crew = Paths::config_dir().join("crew");
    std::fs::create_dir_all(crew.join("credentials")).unwrap();
    let device = device_key().verifying_key().to_bytes();
    let registry = json!({
        "connections": [{
            "id": CONNECTION, "name": "Methods Lab", "ssh_target": "crew@example.test",
            "port": null, "identity_file": null, "proxy_jump": null,
            "socket_path": "/run/crew.sock", "owner_uid": OWNER_UID,
            "workspace_id": WORKSPACE,
            "workspace_public_key": hex(&workspace_key().verifying_key().to_bytes()),
            "remote_root": null, "remote_execution": false,
            "cluster_connection_id": CLUSTER, "mode": "private", "institution_id": INSTITUTION,
            "policy_epoch": 1, "status": "disconnected", "last_error": null,
            "device_id": hex(&Sha256::digest(device)), "public_key": hex(&device),
        }],
        "scopes": {},
    });
    std::fs::write(
        crew.join("connections.json"),
        serde_json::to_vec(&registry).unwrap(),
    )
    .unwrap();
    let credential = crew.join("credentials").join(hex(&Sha256::digest(
        format!("device:{CONNECTION}").as_bytes(),
    )));
    std::fs::write(credential, hex(&device_key().to_bytes())).unwrap();
}

/// A private model affiliated with the workspace's institution that streams every answer in
/// [`CHUNKS`] pieces and, like Llama Server, reports the window it really loaded once it has
/// served a request.
struct StreamingWorker {
    served: AtomicBool,
}

#[async_trait]
impl Provider for StreamingWorker {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new(
            "crew-admission-worker",
            "Crew admission worker",
            "",
            "scripted",
            vec![],
            "",
            vec![],
        )
    }

    fn get_name(&self) -> &str {
        "crew-admission-worker"
    }

    fn tier(&self) -> ProviderTier {
        ProviderTier::Private
    }

    fn affiliation(&self) -> Option<ModelAffiliation> {
        Some(ModelAffiliation::institution(InstitutionId::new(
            INSTITUTION,
        )))
    }

    fn get_model_config(&self) -> ModelConfig {
        let window = if self.served.load(Ordering::SeqCst) {
            LOADED_WINDOW
        } else {
            TABLE_WINDOW
        };
        ModelConfig::new_or_fail("crew-admission-model").with_context_limit(Some(window))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.served.store(true, Ordering::SeqCst);
        Ok((
            Message::assistant().with_text("Methods review"),
            ProviderUsage::new("scripted".into(), Usage::new(Some(1), Some(1), Some(2))),
        ))
    }

    async fn stream(
        &self,
        _system: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        // The server reports its real window as it starts answering.
        self.served.store(true, Ordering::SeqCst);
        let id = format!("streamed-answer-{}", messages.len());
        let mut items: Vec<Result<_, ProviderError>> = (0..CHUNKS)
            .map(|index| {
                Ok((
                    Some(
                        Message::assistant()
                            .with_id(id.clone())
                            .with_text(format!("word{index} ")),
                    ),
                    None,
                    None,
                ))
            })
            .collect();
        items.push(Ok((
            None,
            Some(ProviderUsage::new(
                "scripted".into(),
                Usage::new(Some(10), Some(CHUNKS as i32), Some(10 + CHUNKS as i32)),
            )),
            None,
        )));
        Ok(Box::pin(futures::stream::iter(items)))
    }
}

/// The streamed text of one turn, and every error it ended with.
async fn run_turn(agent: &Agent, session: &str, prompt: &str) -> (String, Vec<String>) {
    let stream = agent
        .reply(
            Message::user().with_text(prompt),
            SessionConfig {
                id: session.to_owned(),
                schedule_id: None,
                max_turns: Some(4),
                max_tool_calls: None,
                budget: None,
                retry_config: None,
                reasoning_effort: None,
            },
            None,
        )
        .await
        .expect("the turn starts under the grant");
    tokio::pin!(stream);
    let mut text = String::new();
    let mut errors = Vec::new();
    while let Some(event) = stream.next().await {
        match event {
            Ok(AgentEvent::Message(message)) => {
                for content in &message.content {
                    if let MessageContent::Text(chunk) = content {
                        text.push_str(&chunk.text);
                    }
                }
            }
            Ok(_) => {}
            Err(error) => errors.push(error.to_string()),
        }
    }
    (text, errors)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_granted_chat_streams_its_answers_with_a_bounded_number_of_admissions() {
    save_the_connection();
    let crew = biorouter::crew::manager().expect("the Crew manager");
    crew.connect(CONNECTION)
        .await
        .expect("the fake broker verifies as the pinned workspace");

    // The person's chat, granted access to #methods by the person, the real way, before its
    // first turn: the provider still reports the model table's window.
    let work_dir = SANDBOX.get().expect("the sandbox").join("work");
    std::fs::create_dir_all(&work_dir).unwrap();
    let sessions = Arc::new(SessionManager::instance());
    let session = sessions
        .create_session(work_dir, "Crew admission".into(), SessionType::User)
        .await
        .unwrap();
    let worker = Arc::new(StreamingWorker {
        served: AtomicBool::new(false),
    });
    crew.grant_session(
        &session.id,
        CONNECTION,
        CHANNEL,
        vec![CHANNEL.to_owned()],
        worker.as_ref(),
        false,
    )
    .await
    .expect("the person's grant");
    assert!(crew.is_scoped_session(&session.id).await);

    let permissions = tempfile::TempDir::new().unwrap();
    let agent = Agent::with_config(AgentConfig::new(
        sessions,
        Arc::new(PermissionManager::new(permissions.path().to_path_buf())),
        None,
        BioRouterMode::Auto,
    ));
    agent
        .update_provider(worker.clone(), &session.id)
        .await
        .expect("a private model binds to the granted chat");
    agent
        .add_extension(ExtensionConfig::Platform {
            name: "crew".into(),
            description: "Crew".into(),
            bundled: Some(true),
            available_tools: vec![],
        })
        .await
        .expect("the Crew tools");

    let expected: String = (0..CHUNKS).map(|index| format!("word{index} ")).collect();
    for turn in ["Summarize the methods thread.", "And the open questions?"] {
        let before = broker_requests("context.manifest");
        let (text, errors) = run_turn(&agent, &session.id, turn).await;
        assert!(
            errors.is_empty(),
            "{turn}: the turn was refused: {errors:?}"
        );
        assert_eq!(text, expected, "{turn}: the whole answer streamed in");
        let admissions = broker_requests("context.manifest") - before;
        // One model request per turn here: the turn-start check and the request's own.
        assert!(
            (1..=2).contains(&admissions),
            "{turn}: {admissions} live admissions for one model request streamed in {CHUNKS} \
             chunks"
        );
    }
    assert_eq!(
        worker.get_model_config().context_limit,
        Some(LOADED_WINDOW),
        "the model reported the window it loaded, so the second turn ran on the moved binding"
    );
    crew.disconnect(CONNECTION).await.unwrap();
}
