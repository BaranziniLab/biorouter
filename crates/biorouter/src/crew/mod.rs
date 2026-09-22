//! Saved native SSH connections and owner-scoped Crew capabilities.
mod transport;
use crate::{
    privacy::{CallCapability, ProviderTier},
    providers::base::Provider,
};
use anyhow::{ensure, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex as StdMutex},
};
use tokio::sync::Mutex;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClusterMode {
    Public,
    #[default]
    Private,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SaveConnection {
    #[serde(default)]
    pub preparation_id: Option<String>,
    pub name: String,
    pub ssh_target: String,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
    pub socket_path: String,
    pub owner_uid: u32,
    pub workspace_id: String,
    pub workspace_public_key: String,
    #[serde(default)]
    pub remote_root: Option<String>,
    #[serde(default)]
    pub remote_execution: bool,
    pub cluster_connection_id: Option<String>,
    #[serde(default)]
    pub mode: ClusterMode,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Connection {
    pub id: String,
    #[serde(default)]
    pub node_id: Option<String>,
    pub name: String,
    pub ssh_target: String,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
    pub socket_path: String,
    pub owner_uid: u32,
    pub workspace_id: String,
    pub workspace_public_key: String,
    #[serde(default)]
    pub remote_root: Option<String>,
    #[serde(default)]
    pub remote_execution: bool,
    pub cluster_connection_id: String,
    pub mode: ClusterMode,
    pub policy_epoch: u64,
    pub status: String,
    pub last_error: Option<String>,
    pub device_id: String,
    pub public_key: String,
}
#[derive(Serialize)]
pub struct AuthenticationPlan {
    pub program: String,
    pub args: Vec<String>,
    pub connection_id: String,
    pub authentication_id: String,
}
#[derive(Clone, Deserialize, Serialize)]
struct Scope {
    connection_id: String,
    run_id: String,
    channel_id: String,
    source_channels: Vec<String>,
    epoch: u64,
    provider_binding: String,
    public_provider: bool,
    #[serde(default)]
    origin_restricted: bool,
    expired: bool,
}
#[derive(Default, Deserialize, Serialize)]
struct Registry {
    connections: Vec<Connection>,
    scopes: HashMap<String, Scope>,
    #[serde(default)]
    pending_device: Option<PreparedDevice>,
    #[serde(default)]
    completed_preparations: HashMap<String, String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PreparedDevice {
    pub preparation_id: String,
    pub public_key: String,
    pub device_id: String,
}
#[derive(Serialize)]
pub struct RunAdmission {
    pub run_id: String,
    pub context: String,
}
pub struct RunMetadata {
    pub run_id: String,
    pub connection_id: String,
    pub channel_id: String,
}
pub struct CrewManager {
    root: PathBuf,
    registry: Mutex<Registry>,
    transports: Mutex<HashMap<String, Arc<Mutex<transport::Transport>>>>,
}
static MANAGERS: LazyLock<StdMutex<HashMap<PathBuf, Arc<CrewManager>>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));
pub fn manager() -> Result<Arc<CrewManager>> {
    let path = crate::config::paths::Paths::config_dir().join("crew");
    let mut managers = MANAGERS
        .lock()
        .map_err(|_| anyhow::anyhow!("Crew registry lock poisoned"))?;
    if let Some(manager) = managers.get(&path) {
        return Ok(manager.clone());
    }
    let manager = Arc::new(CrewManager::new(path.clone())?);
    managers.insert(path, manager.clone());
    Ok(manager)
}
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => serde_json::to_value(
            map.iter()
                .map(|(k, v)| (k.clone(), canonical(v)))
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .expect("JSON values serialize"),
        Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
        _ => value.clone(),
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|v| format!("{v:02x}")).collect()
}
fn unhex(value: &str) -> Result<Vec<u8>> {
    ensure!(
        value.len().is_multiple_of(2) && value.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid encoded key"
    );
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(Into::into))
        .collect()
}
fn provider_binding(provider: &dyn Provider) -> String {
    let resolved =
        serde_json::to_vec(&provider.restore_binding()).expect("provider binding serializes");
    format!(
        "{}:{:?}:{:?}",
        hex(&Sha256::digest(resolved)),
        provider.tier(),
        provider.affiliation()
    )
}
fn file_credentials_enabled() -> bool {
    std::env::var("BIOROUTER_DISABLE_KEYRING").as_deref() == Ok("true")
        && std::env::var_os("BIOROUTER_DEV_PROFILE_ROOT")
            .is_some_and(|p| PathBuf::from(p).is_absolute())
}
pub(super) fn safe_atom(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./:@%-".contains(&b))
        && !value.starts_with('-')
}
impl CrewManager {
    fn credential_path(&self, id: &str) -> PathBuf {
        self.root
            .join("credentials")
            .join(hex(&Sha256::digest(id.as_bytes())))
    }
    fn credential_entry(&self, id: &str) -> Result<keyring::Entry> {
        Ok(keyring::Entry::new(
            "org.biorouter.crew",
            &format!(
                "{}:{id}",
                hex(&Sha256::digest(self.root.to_string_lossy().as_bytes()))
            ),
        )?)
    }
    fn write_credential(&self, id: &str, value: &str) -> Result<()> {
        if file_credentials_enabled() {
            let path = self.credential_path(id);
            let parent = path.parent().expect("credential parent");
            std::fs::create_dir_all(parent)?;
            ensure!(
                !std::fs::symlink_metadata(parent)?.file_type().is_symlink(),
                "Credential directory must not be a symlink"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
            }
            let mut file = tempfile::NamedTempFile::new_in(parent)?;
            use std::io::Write;
            file.write_all(value.as_bytes())?;
            file.as_file().sync_all()?;
            file.persist(path)?;
            Ok(())
        } else {
            self.credential_entry(id)?.set_password(value)?;
            Ok(())
        }
    }
    fn read_credential(&self, id: &str) -> Result<String> {
        if file_credentials_enabled() {
            Ok(std::fs::read_to_string(self.credential_path(id))?)
        } else {
            Ok(self.credential_entry(id)?.get_password()?)
        }
    }
    pub fn new(root: PathBuf) -> Result<Self> {
        let path = root.join("connections.json");
        let mut registry: Registry = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Registry::default(),
            Err(e) => return Err(e.into()),
        };
        for c in &mut registry.connections {
            c.status = "disconnected".into();
            c.last_error = None;
        }
        Ok(Self {
            root,
            registry: Mutex::new(registry),
            transports: Mutex::new(HashMap::new()),
        })
    }
    fn persist(&self, registry: &Registry) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        ensure!(
            !std::fs::symlink_metadata(&self.root)?
                .file_type()
                .is_symlink(),
            "Crew registry must not be a symlink"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))?;
        }
        let mut file = tempfile::NamedTempFile::new_in(&self.root)?;
        use std::io::Write;
        file.write_all(&serde_json::to_vec(registry)?)?;
        file.as_file().sync_all()?;
        file.persist(self.root.join("connections.json"))?;
        Ok(())
    }
    fn control_path(&self, id: &str) -> Result<PathBuf> {
        let root = std::env::temp_dir().join(format!(
            "brcrew-{}",
            &hex(&Sha256::digest(self.root.to_string_lossy().as_bytes()))[..12]
        ));
        std::fs::create_dir_all(&root)?;
        ensure!(
            !std::fs::symlink_metadata(&root)?.file_type().is_symlink(),
            "SSH runtime directory must not be a symlink"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            ensure!(
                std::fs::metadata(&root)?.uid() == unsafe { libc::geteuid() },
                "SSH runtime directory owner mismatch"
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(root.join(id))
    }
    pub async fn list(&self) -> Vec<Connection> {
        self.registry.lock().await.connections.clone()
    }
    pub async fn connection(&self, id: &str) -> Result<Connection> {
        self.registry
            .lock()
            .await
            .connections
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Unknown Crew connection"))
    }
    pub async fn prepare_device(&self) -> Result<PreparedDevice> {
        let mut registry = self.registry.lock().await;
        if let Some(prepared) = &registry.pending_device {
            return Ok(prepared.clone());
        }
        let preparation_id = uuid::Uuid::new_v4().to_string();
        let key = SigningKey::from_bytes(&rand::random::<[u8; 32]>());
        let public = key.verifying_key().to_bytes();
        self.write_credential(&format!("device:{preparation_id}"), &hex(&key.to_bytes()))?;
        let prepared = PreparedDevice {
            preparation_id,
            public_key: hex(&public),
            device_id: hex(&Sha256::digest(public)),
        };
        registry.pending_device = Some(prepared.clone());
        self.persist(&registry)?;
        Ok(prepared)
    }
    pub async fn save(&self, input: SaveConnection) -> Result<Connection> {
        self.save_inner(None, input).await
    }
    pub async fn update(&self, id: &str, input: SaveConnection) -> Result<Connection> {
        self.disconnect(id).await?;
        self.save_inner(Some(id), input).await
    }
    async fn save_inner(&self, id: Option<&str>, input: SaveConnection) -> Result<Connection> {
        ensure!(
            safe_atom(&input.ssh_target),
            "SSH target must be a host alias or user@host"
        );
        ensure!(
            safe_atom(&input.socket_path) && input.socket_path.starts_with('/'),
            "Socket path must be an absolute path without shell syntax"
        );
        uuid::Uuid::parse_str(&input.workspace_id)?;
        ensure!(
            unhex(&input.workspace_public_key)?.len() == 32,
            "Workspace public key must be 32 bytes encoded as hex"
        );
        ensure!(
            input.name.len() <= 120 && !input.name.trim().is_empty(),
            "Connection name must contain 1–120 characters"
        );
        if let Some(jump) = &input.proxy_jump {
            ensure!(jump.split(',').all(safe_atom), "Invalid ProxyJump route");
        }
        if let Some(identity) = &input.identity_file {
            ensure!(
                Path::new(identity).is_absolute() && !identity.contains('\n'),
                "Identity file must be an absolute path"
            );
        }
        let mut r = self.registry.lock().await;
        let preparation_hash = hex(&Sha256::digest(serde_json::to_vec(&input)?));
        let prepared = if let Some(preparation_id) = &input.preparation_id {
            ensure!(
                id.is_none(),
                "Prepared device identities are used only when saving a new connection"
            );
            uuid::Uuid::parse_str(preparation_id)?;
            if let Some(expected) = r.completed_preparations.get(preparation_id) {
                ensure!(
                    expected == &preparation_hash,
                    "Prepared identity was already saved with different connection settings"
                );
                let connection = r
                    .connections
                    .iter()
                    .find(|connection| &connection.id == preparation_id)
                    .cloned()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Prepared connection was removed; prepare a new device identity"
                        )
                    })?;
                self.persist(&r)?;
                return Ok(connection);
            }
            Some(
                r.pending_device
                    .as_ref()
                    .filter(|prepared| &prepared.preparation_id == preparation_id)
                    .cloned()
                    .ok_or_else(|| {
                        anyhow::anyhow!("Prepared identity is unavailable in this profile")
                    })?,
            )
        } else {
            None
        };
        let old = if let Some(id) = id {
            Some(
                r.connections
                    .iter()
                    .find(|c| c.id == id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("Unknown Crew connection"))?,
            )
        } else {
            None
        };
        let connection_id = old
            .as_ref()
            .map(|c| c.id.clone())
            .or_else(|| {
                prepared
                    .as_ref()
                    .map(|prepared| prepared.preparation_id.clone())
            })
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let (device_id, public_key) = if let Some(c) = &old {
            (c.device_id.clone(), c.public_key.clone())
        } else if let Some(prepared) = &prepared {
            let secret: [u8; 32] =
                unhex(&self.read_credential(&format!("device:{connection_id}"))?)?
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("Prepared device key is invalid"))?;
            ensure!(
                hex(&SigningKey::from_bytes(&secret).verifying_key().to_bytes())
                    == prepared.public_key,
                "Prepared public key does not match profile credential"
            );
            (prepared.device_id.clone(), prepared.public_key.clone())
        } else {
            let key = SigningKey::from_bytes(&rand::random::<[u8; 32]>());
            let public = key.verifying_key().to_bytes();
            let device = hex(&Sha256::digest(public));
            self.write_credential(&format!("device:{connection_id}"), &hex(&key.to_bytes()))?;
            (device, hex(&public))
        };
        // Workspace aliases share the most restrictive existing cluster identity automatically.
        let canonical = r
            .connections
            .iter()
            .find(|c| c.workspace_id == input.workspace_id || c.ssh_target == input.ssh_target)
            .map(|c| c.cluster_connection_id.clone());
        let cluster = canonical
            .or_else(|| old.as_ref().map(|c| c.cluster_connection_id.clone()))
            .or(input.cluster_connection_id)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        uuid::Uuid::parse_str(&cluster)?;
        let epoch = r
            .connections
            .iter()
            .filter(|c| c.cluster_connection_id == cluster)
            .map(|c| c.policy_epoch)
            .max()
            .unwrap_or(0)
            + 1;
        let mode = if r.connections.iter().any(|c| {
            c.cluster_connection_id == cluster
                && Some(c.id.as_str()) != id
                && c.mode == ClusterMode::Private
        }) {
            ClusterMode::Private
        } else {
            input.mode
        };
        for c in r
            .connections
            .iter_mut()
            .filter(|c| c.cluster_connection_id == cluster)
        {
            c.mode = mode;
            c.policy_epoch = epoch;
        }
        let c = Connection {
            id: connection_id,
            node_id: old.as_ref().and_then(|c| c.node_id.clone()),
            name: input.name,
            ssh_target: input.ssh_target,
            port: input.port,
            identity_file: input.identity_file,
            proxy_jump: input.proxy_jump,
            socket_path: input.socket_path,
            owner_uid: input.owner_uid,
            workspace_id: input.workspace_id,
            workspace_public_key: input.workspace_public_key,
            remote_root: input.remote_root,
            remote_execution: input.remote_execution,
            cluster_connection_id: cluster,
            mode,
            policy_epoch: epoch,
            status: "disconnected".into(),
            last_error: None,
            device_id,
            public_key,
        };
        r.connections.retain(|old| old.id != c.id);
        r.connections.push(c.clone());
        if let Some(prepared) = prepared {
            r.completed_preparations
                .insert(prepared.preparation_id, preparation_hash);
            r.pending_device = None;
        }
        self.persist(&r)?;
        Ok(c)
    }
    pub async fn remove(&self, id: &str) -> Result<()> {
        self.disconnect(id).await?;
        let mut r = self.registry.lock().await;
        r.connections.retain(|c| c.id != id);
        for scope in r.scopes.values_mut().filter(|s| s.connection_id == id) {
            scope.expired = true;
        }
        self.persist(&r)
    }
    pub async fn authentication_plan(&self, id: &str) -> Result<AuthenticationPlan> {
        let c = self.connection(id).await?;
        let mut args = transport::ssh_args(&c, &self.control_path(id)?);
        args.extend([
            "-M".into(),
            "-N".into(),
            "-o".into(),
            "ControlPersist=600".into(),
            c.ssh_target,
        ]);
        Ok(AuthenticationPlan {
            program: "ssh".into(),
            args,
            connection_id: id.into(),
            authentication_id: uuid::Uuid::new_v4().to_string(),
        })
    }
    pub async fn connect(&self, id: &str) -> Result<Connection> {
        let c = self.connection(id).await?;
        let mut transport = transport::Transport::connect(&c, &self.control_path(id)?).await?;
        let challenge_nonce = uuid::Uuid::new_v4().to_string();
        let hello = transport
            .request(
                "hello",
                json!({"challenge_nonce":challenge_nonce}),
                None,
                None,
                None,
            )
            .await?;
        ensure!(
            hello["workspace_id"].as_str() == Some(&c.workspace_id),
            "Workspace identity mismatch"
        );
        ensure!(
            hello["host_uid"].as_u64() == Some(c.owner_uid as u64),
            "Broker owner identity mismatch"
        );
        ensure!(hello["workspace_public_key"].as_str()==Some(c.workspace_public_key.as_str()), "Workspace public key changed or is missing; verify the workspace descriptor before reconnecting");
        ensure!(
            hello["challenge_nonce"].as_str() == Some(challenge_nonce.as_str()),
            "Workspace identity challenge mismatch"
        );
        let public_key: [u8; 32] = unhex(&c.workspace_public_key)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid workspace key"))?;
        let signature = Signature::from_slice(&unhex(
            hello["signature"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Workspace identity signature missing"))?,
        )?)?;
        let node_id = hello["node_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Verified node identity is missing"))?;
        ensure!(
            node_id.len() == 64
                && node_id
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "Invalid node identity"
        );
        let signed = serde_json::to_vec(&json!([
            c.workspace_id,
            c.owner_uid,
            challenge_nonce,
            c.workspace_public_key,
            node_id
        ]))?;
        VerifyingKey::from_bytes(&public_key)?.verify(&signed, &signature)?;
        let mut registry = self.registry.lock().await;
        let current = registry
            .connections
            .iter()
            .find(|current| current.id == id)
            .ok_or_else(|| anyhow::anyhow!("Connection removed while connecting"))?;
        ensure!(
            current.policy_epoch == c.policy_epoch
                && current.workspace_id == c.workspace_id
                && current.workspace_public_key == c.workspace_public_key
                && current.ssh_target == c.ssh_target,
            "Connection changed while authentication was pending; reconnect"
        );
        if let Some(previous) = &current.node_id {
            ensure!(
                previous == node_id,
                "Verified SSH node identity changed; create a newly verified connection"
            );
        }
        let mut groups: std::collections::BTreeSet<String> = registry
            .connections
            .iter()
            .filter(|entry| entry.node_id.as_deref() == Some(node_id))
            .map(|entry| entry.cluster_connection_id.clone())
            .collect();
        groups.insert(c.cluster_connection_id.clone());
        let canonical = groups.first().cloned().expect("current cluster exists");
        let mode = if registry.connections.iter().any(|entry| {
            groups.contains(&entry.cluster_connection_id) && entry.mode == ClusterMode::Private
        }) {
            ClusterMode::Private
        } else {
            ClusterMode::Public
        };
        let changed = groups.len() > 1 || c.node_id.as_deref() != Some(node_id);
        let epoch = registry
            .connections
            .iter()
            .filter(|entry| groups.contains(&entry.cluster_connection_id))
            .map(|entry| entry.policy_epoch)
            .max()
            .unwrap_or(c.policy_epoch)
            + u64::from(changed);
        for entry in registry
            .connections
            .iter_mut()
            .filter(|entry| groups.contains(&entry.cluster_connection_id))
        {
            entry.cluster_connection_id = canonical.clone();
            entry.mode = mode;
            entry.policy_epoch = epoch;
            if entry.id == id {
                entry.node_id = Some(node_id.into());
                entry.status = "connected".into();
                entry.last_error = None;
            }
        }
        self.persist(&registry)?;
        let connected = registry
            .connections
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .expect("validated connection");
        drop(registry);
        self.transports
            .lock()
            .await
            .insert(id.into(), Arc::new(Mutex::new(transport)));
        Ok(connected)
    }
    pub async fn disconnect(&self, id: &str) -> Result<()> {
        if let Ok(connection) = self.connection(id).await {
            let control = self.control_path(id)?;
            if control.exists() {
                let mut args = transport::ssh_args(&connection, &control);
                args.extend(["-O".into(), "exit".into(), connection.ssh_target]);
                let mut child = tokio::process::Command::new("ssh")
                    .args(args)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .spawn()?;
                let _ = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await;
            }
        }
        let removed = { self.transports.lock().await.remove(id) };
        if let Some(t) = removed {
            t.lock().await.close().await;
        }
        let mut r = self.registry.lock().await;
        if let Some(c) = r.connections.iter_mut().find(|c| c.id == id) {
            c.status = "disconnected".into();
        }
        Ok(())
    }
    async fn transport(&self, id: &str) -> Result<Arc<Mutex<transport::Transport>>> {
        self.transports
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!("Crew connection is disconnected; authenticate and connect in Crew")
            })
    }
    pub async fn human_request(
        &self,
        id: &str,
        method: &str,
        params: Value,
        request_id: Option<String>,
    ) -> Result<Value> {
        ensure!(
            method != "run.create",
            "Agent grants must be created through the trusted provider-bound session action"
        );
        self.signed_request(id, method, params, request_id).await
    }
    async fn signed_request(
        &self,
        id: &str,
        method: &str,
        mut params: Value,
        request_id: Option<String>,
    ) -> Result<Value> {
        ensure!(params.is_object(), "Crew params must be an object");
        let c = self.connection(id).await?;
        if matches!(method, "message.post" | "blob.begin") {
            params["personal_mode"] = json!(c.mode);
        }
        if params.get("idempotency_key").is_none() {
            params["idempotency_key"] = json!(request_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()));
        }
        let t = self.transport(id).await?;
        let mut t = t.lock().await;
        let fresh = self.connection(id).await?;
        ensure!(
            fresh.policy_epoch == c.policy_epoch
                && fresh.mode == c.mode
                && fresh.workspace_id == c.workspace_id
                && fresh.workspace_public_key == c.workspace_public_key,
            "Crew connection policy changed while this action was queued; review and retry"
        );
        let challenge = t
            .request(
                "auth.challenge",
                json!({"device_id":c.device_id}),
                None,
                None,
                None,
            )
            .await?;
        ensure!(
            challenge["workspace_id"].as_str() == Some(&c.workspace_id),
            "Challenge workspace mismatch"
        );
        let nonce = challenge["nonce"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing authentication challenge"))?;
        let fresh = self.connection(id).await?;
        ensure!(
            fresh.policy_epoch == c.policy_epoch && fresh.mode == c.mode,
            "Crew connection policy changed during authentication; review and retry"
        );
        let key = unhex(&self.read_credential(&format!("device:{id}"))?)?;
        let key: [u8; 32] = key
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid device key"))?;
        let bytes = serde_json::to_vec(&json!([
            c.workspace_id,
            challenge["uid"],
            nonce,
            method,
            canonical(&params)
        ]))?;
        let signature = hex(&SigningKey::from_bytes(&key).sign(&bytes).to_bytes());
        t.request(
            method,
            params,
            Some(json!({"device_id":c.device_id,"nonce":nonce,"signature":signature})),
            None,
            request_id,
        )
        .await
    }
    pub async fn run_metadata(&self, session: &str) -> Option<RunMetadata> {
        self.registry
            .lock()
            .await
            .scopes
            .get(session)
            .map(|scope| RunMetadata {
                run_id: scope.run_id.clone(),
                connection_id: scope.connection_id.clone(),
                channel_id: scope.channel_id.clone(),
            })
    }
    pub async fn scoped_session_ids(&self) -> std::collections::HashSet<String> {
        self.registry.lock().await.scopes.keys().cloned().collect()
    }
    pub async fn is_scoped_session(&self, session: &str) -> bool {
        self.is_scoped(session).await
    }
    pub async fn authorize_session_tool(&self, session: &str, name: &str) -> Result<()> {
        if self.is_scoped(session).await {
            ensure!(name.starts_with("crew__") || name.starts_with("todo__"), "Crew-scoped conversations permit only Crew and checklist tools until worker isolation is verified");
        }
        Ok(())
    }
    pub async fn agent_connections(&self, session: &str) -> Result<Value> {
        let s = self.scope(session).await?;
        let c = self.connection(&s.connection_id).await?;
        Ok(
            json!({"connections":[{"id":c.id,"name":c.name,"status":c.status,"mode":c.mode,"workspace_id":c.workspace_id}]}),
        )
    }
    pub async fn is_scoped(&self, session: &str) -> bool {
        self.registry.lock().await.scopes.contains_key(session)
    }
    async fn scope(&self, session: &str) -> Result<Scope> {
        self.registry
            .lock()
            .await
            .scopes
            .get(session)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "This conversation has no human-approved Crew run; grant it from Crew"
                )
            })
    }
    pub async fn check_dispatch(&self, session: &str, cap: &CallCapability) -> Result<()> {
        self.check_tier(session, cap.tier()).await
    }
    async fn check_tier(&self, session: &str, tier: ProviderTier) -> Result<()> {
        let r = self.registry.lock().await;
        let Some(s) = r.scopes.get(session) else {
            return Ok(());
        };
        ensure!(
            !s.expired,
            "Crew run was revoked; request a fresh human grant"
        );
        let c = r
            .connections
            .iter()
            .find(|c| c.id == s.connection_id)
            .ok_or_else(|| anyhow::anyhow!("Crew connection was removed"))?;
        ensure!(
            s.epoch == c.policy_epoch,
            "Crew policy changed; request a fresh human grant"
        );
        ensure!(
            tier != ProviderTier::Public
                || (c.mode == ClusterMode::Public && s.public_provider && !s.origin_restricted),
            "Private Crew context cannot be sent to a public model"
        );
        Ok(())
    }
    pub async fn check_provider_binding(
        &self,
        session: &str,
        provider: &dyn Provider,
    ) -> Result<()> {
        let registry = self.registry.lock().await;
        let Some(scope) = registry.scopes.get(session) else {
            return Ok(());
        };
        ensure!(
            !provider.uses_tool_bridge(),
            "Crew cannot bind a provider with unscoped external tools"
        );
        ensure!(scope.provider_binding==provider_binding(provider),"Crew conversation remains bound to its original resolved provider; start a fresh conversation for another model boundary");
        let connection = registry
            .connections
            .iter()
            .find(|connection| connection.id == scope.connection_id)
            .ok_or_else(|| anyhow::anyhow!("Crew connection was removed"))?;
        ensure!(
            provider.tier() != ProviderTier::Public
                || (connection.mode == ClusterMode::Public
                    && scope.public_provider
                    && !scope.origin_restricted),
            "Private Crew context cannot be bound to a public model"
        );
        Ok(())
    }
    pub async fn check_provider_dispatch(
        &self,
        session: &str,
        provider: &dyn Provider,
    ) -> Result<()> {
        self.check_provider_binding(session, provider).await?;
        self.check_tier(session, provider.tier()).await?;
        if self.is_scoped(session).await {
            self.worker_request(session, "context.manifest", json!({}))
                .await?;
        }
        Ok(())
    }
    pub async fn begin_run(
        &self,
        session: &str,
        id: &str,
        channel: &str,
        sources: Vec<String>,
        provider: &dyn Provider,
    ) -> Result<RunAdmission> {
        self.begin_run_with_origin(session, id, channel, sources, provider, false)
            .await
    }
    #[allow(clippy::too_many_arguments)]
    async fn begin_run_with_origin(
        &self,
        session: &str,
        id: &str,
        channel: &str,
        mut sources: Vec<String>,
        provider: &dyn Provider,
        mut origin_restricted: bool,
    ) -> Result<RunAdmission> {
        ensure!(!provider.uses_tool_bridge(), "Crew cannot admit providers with external tools outside its scoped capability boundary");
        let c = self.connection(id).await?;
        let public = provider.tier() == ProviderTier::Public;
        ensure!(
            !public || c.mode == ClusterMode::Public,
            "Private cluster blocks public models"
        );
        if let Some(previous) = self.registry.lock().await.scopes.get(session).cloned() {
            origin_restricted |= previous.origin_restricted;
            ensure!(previous.connection_id == id && previous.channel_id == channel && previous.provider_binding == provider_binding(provider), "An existing Crew conversation retains its original connection, destination and model boundary; start a fresh conversation for another boundary");
            ensure!(
                !public || previous.public_provider,
                "Private-origin Crew conversation cannot be rebound to a public model"
            );
            for source in previous.source_channels {
                if !sources.contains(&source) {
                    sources.push(source);
                }
            }
        }
        ensure!(
            !public || !origin_restricted,
            "Private-origin local conversation cannot be admitted to a public Crew worker"
        );
        if !sources.iter().any(|s| s == channel) {
            sources.push(channel.into());
        }
        let result=self.signed_request(id,"run.create",json!({"channel_id":channel,"source_channels":sources,"provider_policy_id":provider_binding(provider),"personal_mode":if origin_restricted {ClusterMode::Private}else{c.mode},"public_provider":public,"expires_in":3600,"remote_root":if public {None}else{c.remote_root.clone()},"remote_execution":!public && c.remote_execution}),None).await?;
        let run_id = result["run"]["id"]
            .as_str()
            .or_else(|| result["run"]["run_id"].as_str())
            .ok_or_else(|| anyhow::anyhow!("Broker did not return a run ID"))?
            .to_string();
        let credential = result["credential"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Broker did not return a scoped credential"))?;
        self.write_credential(&format!("run:{session}"), credential)?;
        {
            let mut r = self.registry.lock().await;
            r.scopes.insert(
                session.into(),
                Scope {
                    connection_id: id.into(),
                    run_id: run_id.clone(),
                    channel_id: channel.into(),
                    source_channels: sources,
                    epoch: c.policy_epoch,
                    provider_binding: provider_binding(provider),
                    public_provider: public,
                    origin_restricted,
                    expired: false,
                },
            );
            self.persist(&r)?;
        }
        let context = self
            .worker_request(
                session,
                "messages.history",
                json!({"channel_id":channel,"limit":50,"latest":true}),
            )
            .await?;
        Ok(RunAdmission {
            run_id,
            context: serde_json::to_string(&context)?,
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn grant_session(
        &self,
        session: &str,
        id: &str,
        channel: &str,
        sources: Vec<String>,
        provider: &dyn Provider,
        origin_restricted: bool,
    ) -> Result<RunAdmission> {
        self.begin_run_with_origin(session, id, channel, sources, provider, origin_restricted)
            .await
    }
    pub async fn worker_request(
        &self,
        session: &str,
        method: &str,
        mut params: Value,
    ) -> Result<Value> {
        ensure!(
            [
                "messages.history",
                "messages.search",
                "context.manifest",
                "run.project",
                "blob.read",
                "blob.begin",
                "blob.chunk",
                "blob.finish",
                "remote.list",
                "remote.read",
                "remote.write",
                "remote.hash",
                "remote.execute",
                "remote.job_status",
                "remote.cancel"
            ]
            .contains(&method),
            "Operation unavailable to a scoped Crew worker"
        );
        let s = self.scope(session).await?;
        ensure!(
            !method.starts_with("remote.") || !s.public_provider,
            "Public models cannot access remote files/jobs"
        );
        ensure!(!s.expired, "Crew run revoked");
        let c = self.connection(&s.connection_id).await?;
        ensure!(
            s.epoch == c.policy_epoch,
            "Crew policy changed; obtain a fresh grant"
        );
        ensure!(params.is_object(), "Crew params must be an object");
        if let Some(channel) = params.get("channel_id").and_then(Value::as_str) {
            ensure!(
                s.source_channels.iter().any(|v| v == channel),
                "Channel is outside the approved context scope"
            );
        }
        if method == "run.project" {
            params["run_id"] = json!(s.run_id);
            params["channel_id"] = json!(s.channel_id);
            if params.get("idempotency_key").is_none() {
                params["idempotency_key"] = json!(uuid::Uuid::new_v4().to_string());
            }
        }
        let credential = self.read_credential(&format!("run:{session}"))?;
        self.transport(&s.connection_id)
            .await?
            .lock()
            .await
            .request(method, params, None, Some(&credential), None)
            .await
    }
    pub async fn publish_run(&self, session: &str, body: &str, status: &str) -> Result<Value> {
        self.worker_request(session, "run.project", json!({"body":body,"status":status}))
            .await
    }
    pub async fn cancel_run_if_current(
        &self,
        session: &str,
        expected_run_id: &str,
    ) -> Result<Value> {
        let scope = self.scope(session).await?;
        ensure!(scope.run_id==expected_run_id,"This task was replaced by a newer explicitly granted run; cancel it from its current conversation");
        self.revoke_scope(session, scope).await
    }
    pub async fn cancel_run(&self, session: &str) -> Result<Value> {
        self.revoke_scope(session, self.scope(session).await?).await
    }
    async fn revoke_scope(&self, session: &str, s: Scope) -> Result<Value> {
        let result = self
            .human_request(
                &s.connection_id,
                "run.revoke",
                json!({"run_id":s.run_id}),
                None,
            )
            .await?;
        let mut r = self.registry.lock().await;
        if let Some(current) = r
            .scopes
            .get_mut(session)
            .filter(|current| current.run_id == s.run_id)
        {
            current.expired = true;
        }
        self.persist(&r)?;
        Ok(result)
    }
    async fn attach_remote(&self, session: &str, params: Value) -> Result<Value> {
        let scope = self.scope(session).await?;
        ensure!(
            !scope.public_provider,
            "Public models cannot attach remote files"
        );
        let key = params["idempotency_key"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("idempotency_key is required for remote.attach"))?;
        let path = params["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("path is required"))?;
        let file = self
            .worker_request(session, "remote.read", json!({"path":path}))
            .await?;
        let name = Path::new(path)
            .file_name()
            .and_then(|p| p.to_str())
            .ok_or_else(|| anyhow::anyhow!("attachment filename unavailable"))?;
        let blob=self.worker_request(session,"blob.begin",json!({"channel_id":scope.channel_id,"name":name,"media_type":params.get("media_type").and_then(Value::as_str).unwrap_or("application/octet-stream"),"size":file["size"],"sha256":file["sha256"],"personal_mode":"private","idempotency_key":format!("{key}:begin")})).await?;
        let blob_id = blob["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("attachment ID missing"))?;
        if file["size"].as_u64().unwrap_or(0) > 0 {
            self.worker_request(session,"blob.chunk",json!({"blob_id":blob_id,"offset":0,"data_hex":file["data_hex"],"idempotency_key":format!("{key}:chunk")})).await?;
        }
        self.worker_request(
            session,
            "blob.finish",
            json!({"blob_id":blob_id,"idempotency_key":format!("{key}:finish")}),
        )
        .await?;
        self.worker_request(session,"run.project",json!({"body":format!("Attached {name}"),"status":"progress","attachments":[blob_id],"idempotency_key":format!("{key}:post")})).await
    }
    pub async fn agent_request(
        &self,
        session: &str,
        cap: &CallCapability,
        id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        self.check_dispatch(session, cap).await?;
        let s = self.scope(session).await?;
        ensure!(
            s.connection_id == id,
            "Connection is outside the approved run scope"
        );
        if method == "run.project" {
            ensure!(
                params.get("status").and_then(Value::as_str).is_none_or(|status| status == "progress"),
                "Agent updates must use progress; the task owner or runner controls completion and cancellation"
            );
            let mut params = params;
            ensure!(params.is_object(), "Crew params must be an object");
            params["status"] = json!("progress");
            return self.worker_request(session, method, params).await;
        }
        if method == "remote.attach" {
            return self.attach_remote(session, params).await;
        }
        self.worker_request(session, method, params).await
    }
}
