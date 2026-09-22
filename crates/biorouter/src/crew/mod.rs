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
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
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
#[derive(Clone, Default, Deserialize, Serialize)]
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
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Ok(u8::from_str_radix(std::str::from_utf8(pair)?, 16)?))
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
        #[cfg(unix)]
        let directories_to_sync = {
            let mut directories = vec![self.root.clone()];
            let mut directory = self.root.as_path();
            while !directory.exists() {
                directory = directory
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("Crew registry parent unavailable"))?;
                directories.push(directory.to_path_buf());
            }
            directories
        };
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
        #[cfg(unix)]
        for directory in directories_to_sync {
            std::fs::File::open(directory)?.sync_all()?;
        }
        Ok(())
    }
    fn control_path(&self, id: &str) -> Result<PathBuf> {
        #[cfg(unix)]
        {
            use std::os::unix::{
                ffi::OsStrExt,
                fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt},
            };
            // Darwin's per-user TMPDIR can consume most of sockaddr_un before
            // the socket name is appended. A private directory under the short,
            // verified system temporary directory keeps OpenSSH portable.
            let temporary = std::fs::canonicalize("/tmp")?;
            let parent = std::fs::symlink_metadata(&temporary)?;
            let uid = unsafe { libc::geteuid() };
            ensure!(
                parent.is_dir()
                    && ((parent.uid() == 0 && parent.mode() & 0o1000 != 0)
                        || (parent.uid() == uid && parent.mode() & 0o077 == 0)),
                "SSH temporary directory must be root-owned and sticky or private to this user"
            );
            let mut namespace = Sha256::new();
            namespace.update(uid.to_be_bytes());
            namespace.update([0]);
            namespace.update(self.root.as_os_str().as_bytes());
            let profile = hex(&namespace.finalize()[..16]);
            let root = temporary.join(format!("brc-{profile}"));
            match std::fs::DirBuilder::new().mode(0o700).create(&root) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            let directory = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&root)?;
            let metadata = directory.metadata()?;
            ensure!(
                metadata.uid() == uid && metadata.mode() & 0o077 == 0,
                "SSH runtime directory must be private and owned by this user"
            );
            let path = root.join(hex(&Sha256::digest(id.as_bytes())[..16]));
            // OpenSSH first binds a temporary socket with a dot and sixteen
            // random characters. Reserve those seventeen bytes plus the NUL
            // terminator within Darwin's 104-byte sockaddr_un.sun_path.
            ensure!(
                path.as_os_str().as_bytes().len() <= 86,
                "SSH control socket path exceeds the portable length limit including its temporary suffix"
            );
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) => ensure!(
                    metadata.file_type().is_socket() && metadata.uid() == uid,
                    "SSH control path must be an owned socket, not a symlink or another file"
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            Ok(path)
        }
        #[cfg(not(unix))]
        {
            let root = std::env::temp_dir().join(format!(
                "brcrew-{}",
                hex(&Sha256::digest(self.root.to_string_lossy().as_bytes())[..6])
            ));
            std::fs::create_dir_all(&root)?;
            ensure!(
                !std::fs::symlink_metadata(&root)?.file_type().is_symlink(),
                "SSH runtime directory must not be a symlink"
            );
            Ok(root.join(id))
        }
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
    fn validate_connection(input: &SaveConnection) -> Result<()> {
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
        Ok(())
    }
    fn connection_device(
        &self,
        connection_id: &str,
        old: Option<&Connection>,
        prepared: Option<&PreparedDevice>,
    ) -> Result<(String, String)> {
        Ok(if let Some(c) = old {
            (c.device_id.clone(), c.public_key.clone())
        } else if let Some(prepared) = prepared {
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
        })
    }
    fn build_connection(
        &self,
        r: &mut Registry,
        id: Option<&str>,
        input: SaveConnection,
        prepared: Option<&PreparedDevice>,
    ) -> Result<Connection> {
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
        let (device_id, public_key) =
            self.connection_device(&connection_id, old.as_ref(), prepared)?;
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
        Ok(Connection {
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
        })
    }
    async fn save_inner(&self, id: Option<&str>, input: SaveConnection) -> Result<Connection> {
        Self::validate_connection(&input)?;
        let mut registry = self.registry.lock().await;
        let mut r = registry.clone();
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
        let c = self.build_connection(&mut r, id, input, prepared.as_ref())?;
        r.connections.retain(|old| old.id != c.id);
        r.connections.push(c.clone());
        if let Some(prepared) = prepared {
            r.completed_preparations
                .insert(prepared.preparation_id, preparation_hash);
            r.pending_device = None;
        }
        self.persist(&r)?;
        *registry = r;
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
    fn verify_workspace_identity(
        c: &Connection,
        hello: &Value,
        challenge_nonce: &str,
    ) -> Result<String> {
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
            hello["challenge_nonce"].as_str() == Some(challenge_nonce),
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
        Ok(node_id.to_string())
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
        let node_id = Self::verify_workspace_identity(&c, &hello, &challenge_nonce)?;
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
                previous == &node_id,
                "Verified SSH node identity changed; create a newly verified connection"
            );
        }
        let mut groups: std::collections::BTreeSet<String> = registry
            .connections
            .iter()
            .filter(|entry| entry.node_id.as_deref() == Some(node_id.as_str()))
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
        let changed = groups.len() > 1 || c.node_id.as_deref() != Some(node_id.as_str());
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
                entry.node_id = Some(node_id.clone());
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
        let key: [u8; 32] = unhex(&self.read_credential(&format!("device:{id}"))?)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid device key"))?;
        let signer = SigningKey::from_bytes(&key);
        let public = signer.verifying_key().to_bytes();
        ensure!(
            hex(&public) == c.public_key && hex(&Sha256::digest(public)) == c.device_id,
            "Saved Crew device identity does not match its signing credential; reconnect using a verified device identity"
        );
        if matches!(method, "auth.bootstrap" | "auth.enroll") {
            ensure!(
                params["public_key"].as_str() == Some(c.public_key.as_str()),
                "Enrollment identity changed; refresh the saved connection before joining"
            );
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
        let bytes = serde_json::to_vec(&json!([
            c.workspace_id,
            challenge["uid"],
            nonce,
            method,
            canonical(&params)
        ]))?;
        let signature = hex(&signer.sign(&bytes).to_bytes());
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
            json!({"connections":[{"id":c.id,"name":c.name,"status":c.status,"mode":c.mode,"workspace_id":c.workspace_id,"destination_channel_id":s.channel_id,"remote_files_enabled":!s.public_provider && c.remote_root.is_some(),"remote_execution_enabled":!s.public_provider && c.remote_root.is_some() && c.remote_execution,"remote_path_base":"the granted SSH work directory, not the local task directory; supply relative paths"}]}),
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
            context: serde_json::to_string(&json!({
                "connection_id": id,
                "destination_channel_id": channel,
                "remote_files_enabled": !public && c.remote_root.is_some(),
                "remote_path_base": "the granted SSH work directory; use relative paths such as crew-task.csv, never the local task working directory",
                "history": context
            }))?,
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
        let transport = self.transport(&s.connection_id).await?;
        let mut transport = transport.lock().await;
        self.validate_worker_scope(session, &s, &c).await?;
        let credential = self.read_credential(&format!("run:{session}"))?;
        let result = transport
            .request(method, params, None, Some(&credential), None)
            .await?;
        self.validate_worker_scope(session, &s, &c).await?;
        Ok(result)
    }
    async fn validate_worker_scope(
        &self,
        session: &str,
        expected_scope: &Scope,
        expected_connection: &Connection,
    ) -> Result<()> {
        let registry = self.registry.lock().await;
        let scope = registry
            .scopes
            .get(session)
            .ok_or_else(|| anyhow::anyhow!("Crew run is unavailable"))?;
        let connection = registry
            .connections
            .iter()
            .find(|connection| connection.id == scope.connection_id)
            .ok_or_else(|| anyhow::anyhow!("Crew connection was removed"))?;
        ensure!(
            scope == expected_scope
                && !scope.expired
                && scope.epoch == connection.policy_epoch
                && connection.policy_epoch == expected_connection.policy_epoch
                && connection.mode == expected_connection.mode
                && connection.workspace_id == expected_connection.workspace_id
                && connection.workspace_public_key == expected_connection.workspace_public_key
                && (!scope.public_provider
                    || (connection.mode == ClusterMode::Public && !scope.origin_restricted)),
            "Crew grant or connection policy changed while this operation was pending; inspect any submitted effects before obtaining a fresh grant"
        );
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agents::{
            crew_extension::CrewClient,
            extension::PlatformExtensionContext,
            mcp_client::{McpClientTrait, McpMeta},
        },
        privacy::{CallCapability, ProviderTier},
        session::SessionManager,
    };
    use rmcp::model::CallToolResult;
    use std::{
        fs,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };
    use tokio_util::sync::CancellationToken;

    fn fixture_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "biorouter-crew-manager-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[tokio::test]
    async fn failed_policy_save_preserves_connection_and_scope_authority() {
        let root = fixture_root("failed-policy-save");
        let connection_id = "11111111-1111-4111-8111-111111111111".to_owned();
        let cluster_id = "22222222-2222-4222-8222-222222222222".to_owned();
        let workspace_id = "33333333-3333-4333-8333-333333333333".to_owned();
        let connection = Connection {
            id: connection_id.clone(),
            node_id: None,
            name: "fixture".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id: workspace_id.clone(),
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: cluster_id.clone(),
            mode: ClusterMode::Private,
            policy_epoch: 7,
            status: "disconnected".into(),
            last_error: None,
            device_id: "22".repeat(32),
            public_key: "33".repeat(32),
        };
        let scope = Scope {
            connection_id: connection_id.clone(),
            run_id: "run-1".into(),
            channel_id: "channel-1".into(),
            source_channels: vec!["channel-1".into()],
            epoch: 7,
            provider_binding: "private".into(),
            public_provider: false,
            origin_restricted: false,
            expired: false,
        };
        let registry = Registry {
            connections: vec![connection],
            scopes: HashMap::from([("session-1".into(), scope.clone())]),
            pending_device: None,
            completed_preparations: HashMap::new(),
        };
        fs::write(
            root.join("connections.json"),
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        let manager = CrewManager::new(root.clone()).unwrap();
        fs::remove_file(root.join("connections.json")).unwrap();
        fs::create_dir(root.join("connections.json")).unwrap();

        let input = SaveConnection {
            preparation_id: None,
            name: "fixture-public-alias".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id,
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: Some(cluster_id),
            mode: ClusterMode::Public,
        };
        assert!(manager
            .save_inner(Some(&connection_id), input)
            .await
            .is_err());
        let live = manager.connection(&connection_id).await.unwrap();
        assert_eq!(live.mode, ClusterMode::Private);
        assert_eq!(live.policy_epoch, 7);
        let live_scope = manager
            .registry
            .lock()
            .await
            .scopes
            .get("session-1")
            .cloned()
            .unwrap();
        assert!(live_scope == scope);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn worker_scope_validation_rejects_alias_epoch_and_expiry_then_accepts_regrant() {
        let root = fixture_root("worker-scope-validation");
        let connection_id = "44444444-4444-4444-8444-444444444444".to_owned();
        let workspace_id = "55555555-5555-4555-8555-555555555555".to_owned();
        let connection = Connection {
            id: connection_id.clone(),
            node_id: None,
            name: "fixture".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id,
            workspace_public_key: "44".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: "66666666-6666-4666-8666-666666666666".into(),
            mode: ClusterMode::Public,
            policy_epoch: 4,
            status: "connected".into(),
            last_error: None,
            device_id: "55".repeat(32),
            public_key: "66".repeat(32),
        };
        let scope = Scope {
            connection_id: connection_id.clone(),
            run_id: "run-public".into(),
            channel_id: "channel-public".into(),
            source_channels: vec!["channel-public".into()],
            epoch: 4,
            provider_binding: "public".into(),
            public_provider: true,
            origin_restricted: false,
            expired: false,
        };
        let registry = Registry {
            connections: vec![connection.clone()],
            scopes: HashMap::from([("session-public".into(), scope.clone())]),
            pending_device: None,
            completed_preparations: HashMap::new(),
        };
        fs::write(
            root.join("connections.json"),
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        let manager = CrewManager::new(root.clone()).unwrap();
        assert!(manager
            .validate_worker_scope("session-public", &scope, &connection)
            .await
            .is_ok());

        let mut wrong_epoch = scope.clone();
        wrong_epoch.epoch = 3;
        assert!(manager
            .validate_worker_scope("session-public", &wrong_epoch, &connection)
            .await
            .is_err());
        let mut expired = scope.clone();
        expired.expired = true;
        assert!(manager
            .validate_worker_scope("session-public", &expired, &connection)
            .await
            .is_err());

        let mut aliased = connection.clone();
        aliased.mode = ClusterMode::Private;
        aliased.policy_epoch = 5;
        manager.registry.lock().await.connections[0] = aliased.clone();
        assert!(manager
            .validate_worker_scope("session-public", &scope, &connection)
            .await
            .is_err());

        let mut regranted = scope;
        regranted.run_id = "run-private-regrant".into();
        regranted.epoch = 5;
        regranted.public_provider = false;
        manager
            .registry
            .lock()
            .await
            .scopes
            .insert("session-public".into(), regranted.clone());
        assert!(manager
            .validate_worker_scope("session-public", &regranted, &aliased)
            .await
            .is_ok());
        let _ = fs::remove_dir_all(root);
    }

    fn result_text(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|content| content.as_text().map(|text| text.text.clone()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn crew_client(label: &str) -> CrewClient {
        CrewClient::new(PlatformExtensionContext {
            extension_manager: None,
            session_manager: Arc::new(SessionManager::new(fixture_root(label))),
        })
    }

    #[tokio::test]
    async fn crew_extension_rejects_invalid_explicit_connection_id_shapes() {
        let _root = crate::test_sandbox::pin_sandbox_path_root();
        let client = crew_client("invalid-connection-id-session");
        let meta = McpMeta::new(
            "invalid-connection-id-session",
            CallCapability::for_test(ProviderTier::Private, true),
        );

        for connection_id in [Value::Null, json!(7), json!({"id": "connection"})] {
            let result = client
                .call_tool(
                    "request",
                    Some(
                        json!({
                            "connection_id": connection_id,
                            "method": "remote.list"
                        })
                        .as_object()
                        .cloned()
                        .unwrap(),
                    ),
                    meta.clone(),
                    CancellationToken::new(),
                )
                .await
                .unwrap();
            assert!(
                result_text(&result).contains("connection_id must be a string"),
                "unexpected refusal: {}",
                result_text(&result)
            );
        }
    }

    #[tokio::test]
    async fn crew_extension_omitted_connection_id_refuses_without_a_grant() {
        let _root = crate::test_sandbox::pin_sandbox_path_root();
        let client = crew_client("omitted-connection-id-without-grant-session");
        let result = client
            .call_tool(
                "request",
                Some(
                    json!({"method": "remote.list"})
                        .as_object()
                        .cloned()
                        .unwrap(),
                ),
                McpMeta::new(
                    "omitted-connection-id-without-grant-session",
                    CallCapability::for_test(ProviderTier::Private, true),
                ),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(result_text(&result).contains("Request a Crew grant"));
    }

    #[tokio::test]
    async fn agent_request_rejects_an_explicit_connection_outside_the_run_scope() {
        let root = fixture_root("wrong-connection-id");
        let connection_id = "77777777-7777-4777-8777-777777777777".to_owned();
        let connection = Connection {
            id: connection_id.clone(),
            node_id: None,
            name: "fixture".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id: "88888888-8888-4888-8888-888888888888".into(),
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: "99999999-9999-4999-8999-999999999999".into(),
            mode: ClusterMode::Private,
            policy_epoch: 1,
            status: "connected".into(),
            last_error: None,
            device_id: "22".repeat(32),
            public_key: "33".repeat(32),
        };
        let scope = Scope {
            connection_id: connection_id.clone(),
            run_id: "run-1".into(),
            channel_id: "channel-1".into(),
            source_channels: vec!["channel-1".into()],
            epoch: 1,
            provider_binding: "private".into(),
            public_provider: false,
            origin_restricted: false,
            expired: false,
        };
        let registry = Registry {
            connections: vec![connection],
            scopes: HashMap::from([("session-1".into(), scope)]),
            pending_device: None,
            completed_preparations: HashMap::new(),
        };
        fs::write(
            root.join("connections.json"),
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        let manager = CrewManager::new(root.clone()).unwrap();
        let error = manager
            .agent_request(
                "session-1",
                &CallCapability::for_test(ProviderTier::Private, true),
                "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
                "remote.list",
                json!({}),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("outside the approved run scope"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn control_path_uses_short_system_tmp_even_when_tmpdir_is_long() {
        use std::os::unix::fs::MetadataExt;

        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("control-path-length");
        let long_tmpdir = root.join("a".repeat(160));
        fs::create_dir_all(&long_tmpdir).unwrap();
        let long_tmpdir_string = long_tmpdir.to_string_lossy().into_owned();
        let _env = crate::test_sandbox::relocate_path_root_and(
            crate::test_sandbox::sandbox_path_root(),
            [("TMPDIR", Some(long_tmpdir_string.as_str()))],
        );
        let manager = CrewManager::new(root.clone()).unwrap();
        let path = manager.control_path("length-check").unwrap();
        assert!(path.starts_with(std::fs::canonicalize("/tmp").unwrap()));
        assert!(path.as_os_str().len() <= 86);
        let metadata = fs::metadata(path.parent().unwrap()).unwrap();
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.mode() & 0o077, 0);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn control_path_rejects_a_symlink_collision() {
        use std::os::unix::fs::symlink;

        let root = fixture_root("control-path-symlink");
        let manager = CrewManager::new(root.clone()).unwrap();
        let path = manager.control_path("symlink-check").unwrap();
        symlink("/tmp/does-not-exist", &path).unwrap();
        let error = manager
            .control_path("symlink-check")
            .unwrap_err()
            .to_string();
        assert!(error.contains("owned socket"), "{error}");
        fs::remove_file(&path).unwrap();
        fs::remove_dir(path.parent().unwrap()).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn control_path_runtime_directory_is_private_and_owned() {
        use std::os::unix::fs::MetadataExt;

        let root = fixture_root("control-path-permissions");
        let manager = CrewManager::new(root.clone()).unwrap();
        let path = manager.control_path("permission-check").unwrap();
        let metadata = fs::metadata(path.parent().unwrap()).unwrap();
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.mode() & 0o077, 0);
        assert!(!fs::symlink_metadata(path.parent().unwrap())
            .unwrap()
            .file_type()
            .is_symlink());
        let _ = fs::remove_dir(path.parent().unwrap());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn crew_extension_omitted_connection_id_reaches_the_bound_request_target() {
        use std::os::unix::fs::PermissionsExt;

        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("omitted-connection-id-target");
        let profile_root = root.join("profile");
        let fake_bin = root.join("bin");
        fs::create_dir_all(&fake_bin).unwrap();
        let workspace_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let ssh = format!(
            r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  if printf '%s' "$line" | grep -q 'auth.challenge'; then
    printf '{{"id":"%s","result":{{"workspace_id":"{workspace_id}","nonce":"nonce"}}}}\n' "$id"
  else
    printf '{{"id":"%s","result":{{"accepted_method":"remote.list"}}}}\n' "$id"
  fi
done
"#
        );
        let ssh_path = fake_bin.join("ssh");
        fs::write(&ssh_path, ssh).unwrap();
        fs::set_permissions(&ssh_path, fs::Permissions::from_mode(0o700)).unwrap();
        let original_path = std::env::var("PATH").unwrap_or_default();
        let path = format!("{}:{original_path}", fake_bin.display());
        let profile_string = profile_root.to_string_lossy().into_owned();
        let _env = crate::test_sandbox::relocate_path_root_and(
            profile_string.as_str(),
            [
                ("BIOROUTER_DEV_PROFILE_ROOT", Some(profile_string.as_str())),
                ("BIOROUTER_DISABLE_KEYRING", Some("true")),
                ("PATH", Some(path.as_str())),
            ],
        );
        let manager_root = crate::config::paths::Paths::config_dir().join("crew");
        fs::create_dir_all(&manager_root).unwrap();
        let connection_id = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".to_owned();
        let device_key = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
        let connection = Connection {
            id: connection_id.clone(),
            node_id: None,
            name: "fixture".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id: workspace_id.into(),
            workspace_public_key: "11".repeat(32),
            remote_root: Some("/srv/crew".into()),
            remote_execution: true,
            cluster_connection_id: "cccccccc-cccc-4ccc-8ccc-cccccccccccc".into(),
            mode: ClusterMode::Private,
            policy_epoch: 1,
            status: "connected".into(),
            last_error: None,
            device_id: hex(&Sha256::digest(device_key.verifying_key().to_bytes())),
            public_key: hex(&device_key.verifying_key().to_bytes()),
        };
        let registry = Registry {
            connections: vec![connection.clone()],
            scopes: HashMap::from([(
                "session-bound-target".into(),
                Scope {
                    connection_id: connection_id.clone(),
                    run_id: "run-bound-target".into(),
                    channel_id: "channel-bound-target".into(),
                    source_channels: vec!["channel-bound-target".into()],
                    epoch: 1,
                    provider_binding: "private".into(),
                    public_provider: false,
                    origin_restricted: false,
                    expired: false,
                },
            )]),
            pending_device: None,
            completed_preparations: HashMap::new(),
        };
        fs::write(
            manager_root.join("connections.json"),
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        let manager = manager().unwrap();
        manager
            .write_credential(
                &format!("device:{connection_id}"),
                &hex(&device_key.to_bytes()),
            )
            .unwrap();
        manager
            .write_credential("run:session-bound-target", "run-credential")
            .unwrap();
        let control = manager.control_path(&connection_id).unwrap();
        let transport = transport::Transport::connect(&connection, &control)
            .await
            .unwrap();
        manager
            .transports
            .lock()
            .await
            .insert(connection_id.clone(), Arc::new(Mutex::new(transport)));

        let client = crew_client("omitted-connection-id-target-session");
        let result = client
            .call_tool(
                "request",
                Some(
                    json!({"method": "remote.list"})
                        .as_object()
                        .cloned()
                        .unwrap(),
                ),
                McpMeta::new(
                    "session-bound-target",
                    CallCapability::for_test(ProviderTier::Private, true),
                ),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(result_text(&result), r#"{"accepted_method":"remote.list"}"#);

        manager.disconnect(&connection_id).await.unwrap();
        let _ = fs::remove_dir_all(root);
    }
}
