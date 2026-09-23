//! Saved native SSH connections and owner-scoped Crew capabilities.
pub mod authentication;
mod credentials;
pub mod observation;
pub use credentials::CredentialStatus;
mod ssh_policy;
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, utoipa::ToSchema)]
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
#[derive(Default)]
pub struct RunPolicy {
    pub origin_restricted: bool,
    pub expected_mode: Option<ClusterMode>,
}
pub struct RunMetadata {
    pub run_id: String,
    pub connection_id: String,
    pub channel_id: String,
}
pub struct CrewManager {
    root: PathBuf,
    credential_vault: Arc<credentials::CredentialVault>,
    registry: Mutex<Registry>,
    transports: Mutex<HashMap<String, Arc<Mutex<transport::Transport>>>>,
    lifecycle: StdMutex<HashMap<String, std::sync::Weak<Mutex<()>>>>,
}
pub(super) fn connection_binding(connection: &Connection) -> Result<Value> {
    let mut value = serde_json::to_value(connection)?;
    if let Some(fields) = value.as_object_mut() {
        fields.remove("status");
        fields.remove("last_error");
    }
    Ok(value)
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

#[cfg(test)]
pub(crate) async fn install_test_scope(
    session_id: &str,
    provider: Option<&dyn Provider>,
) -> Arc<CrewManager> {
    let manager = manager().expect("the test Crew manager should initialize");
    let connection_id = format!("context-fixture-connection-{session_id}");
    let mut registry = manager.registry.lock().await;
    registry.connections.push(Connection {
        id: connection_id.clone(),
        node_id: None,
        name: "context fixture".into(),
        ssh_target: "fixture@example.test".into(),
        port: Some(22),
        identity_file: None,
        proxy_jump: None,
        socket_path: "/tmp/context-fixture.sock".into(),
        owner_uid: 10001,
        workspace_id: format!("workspace-{session_id}"),
        workspace_public_key: "11".repeat(32),
        remote_root: None,
        remote_execution: false,
        cluster_connection_id: format!("cluster-{session_id}"),
        mode: ClusterMode::Public,
        policy_epoch: 1,
        status: "connected".into(),
        last_error: None,
        device_id: "22".repeat(32),
        public_key: "33".repeat(32),
    });
    registry.scopes.insert(
        session_id.into(),
        Scope {
            connection_id,
            run_id: format!("run-{session_id}"),
            channel_id: format!("channel-{session_id}"),
            source_channels: vec![],
            epoch: 1,
            provider_binding: provider.map_or_else(|| "test-context".into(), provider_binding),
            public_provider: true,
            origin_restricted: false,
            expired: false,
        },
    );
    drop(registry);
    manager
}

#[cfg(test)]
pub(crate) async fn remove_test_scope(manager: &CrewManager, session_id: &str) {
    let mut registry = manager.registry.lock().await;
    registry.scopes.remove(session_id);
    registry
        .connections
        .retain(|connection| connection.id != format!("context-fixture-connection-{session_id}"));
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
        self.credential_vault
            .write(id, value, || self.write_legacy_credential(id, value))
    }
    fn write_legacy_credential(&self, id: &str, value: &str) -> Result<()> {
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
    fn read_credential(&self, id: &str) -> Result<zeroize::Zeroizing<String>> {
        self.credential_vault.read(id, || {
            self.read_legacy_credential(id).map(zeroize::Zeroizing::new)
        })
    }
    fn read_legacy_credential(&self, id: &str) -> Result<String> {
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
            credential_vault: Arc::new(credentials::CredentialVault::new(root.clone())),
            root,
            registry: Mutex::new(registry),
            transports: Mutex::new(HashMap::new()),
            lifecycle: StdMutex::new(HashMap::new()),
        })
    }
    pub async fn credential_status(&self) -> Result<CredentialStatus> {
        let vault = self.credential_vault.clone();
        tokio::task::spawn_blocking(move || vault.status()).await?
    }
    pub async fn init_vault(&self, passphrase: zeroize::Zeroizing<String>) -> Result<()> {
        ensure!(!file_credentials_enabled(), "Encrypted vault initialization requires a production credential profile, not the development plaintext backend");
        let registry = self.registry.lock().await;
        ensure!(registry.connections.is_empty() && registry.scopes.is_empty() && registry.pending_device.is_none() && registry.completed_preparations.is_empty(), "Initialize an encrypted vault in a fresh Crew profile before creating identities; existing keyring credentials are never silently replaced");
        let vault = self.credential_vault.clone();
        let result = tokio::task::spawn_blocking(move || vault.init(passphrase)).await?;
        drop(registry);
        result
    }
    pub async fn unlock_vault(&self, passphrase: zeroize::Zeroizing<String>) -> Result<()> {
        let vault = self.credential_vault.clone();
        tokio::task::spawn_blocking(move || vault.unlock(passphrase)).await?
    }
    pub async fn lock_vault(&self) -> Result<()> {
        let vault = self.credential_vault.clone();
        tokio::task::spawn_blocking(move || vault.lock()).await?
    }
    pub async fn session_grants(&self, connection_id: &str) -> Result<Value> {
        self.connection(connection_id).await?;
        let registry = self.registry.lock().await;
        Ok(
            json!({"grants": registry.scopes.iter().filter(|(_, scope)| scope.connection_id == connection_id).map(|(session, scope)| json!({"session_id":session,"run_id":scope.run_id,"connection_id":scope.connection_id,"channel_id":scope.channel_id,"source_channels":scope.source_channels,"policy_epoch":scope.epoch,"expired":scope.expired})).collect::<Vec<_>>()}),
        )
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
        let _lifecycle = self.connection_guard(id).await?;
        self.disconnect_locked(id).await?;
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
        let _lifecycle = self.connection_guard(id).await?;
        self.disconnect_locked(id).await?;
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
            c.ssh_target.clone(),
        ]);
        ssh_policy::preflight(&args, &c.ssh_target).await?;
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
    pub async fn connection_guard(&self, id: &str) -> Result<tokio::sync::OwnedMutexGuard<()>> {
        let lock = {
            let mut locks = self
                .lifecycle
                .lock()
                .map_err(|_| anyhow::anyhow!("Connection lifecycle unavailable"))?;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = locks.get(id).and_then(std::sync::Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(Mutex::new(()));
                locks.insert(id.into(), Arc::downgrade(&lock));
                lock
            }
        };
        Ok(lock.lock_owned().await)
    }
    pub async fn connect(&self, id: &str) -> Result<Connection> {
        let _lifecycle = self.connection_guard(id).await?;
        authentication::ensure_connect_available(id)?;
        self.connect_locked(id).await
    }
    pub(super) async fn connect_locked(&self, id: &str) -> Result<Connection> {
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
            connection_binding(current)? == connection_binding(&c)?,
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
        let _lifecycle = self.connection_guard(id).await?;
        self.disconnect_locked(id).await
    }
    pub(super) async fn disconnect_locked(&self, id: &str) -> Result<()> {
        authentication::cancel_connection(id);
        if let Ok(connection) = self.connection(id).await {
            let control = self.control_path(id)?;
            if control.exists() {
                let mut args = transport::ssh_args(&connection, &control);
                args.extend(["-O".into(), "exit".into(), connection.ssh_target]);
                let mut command = tokio::process::Command::new("ssh");
                command
                    .args(args)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true);
                crate::subprocess::prepare_agent_child_command(&mut command);
                let mut child = command.spawn()?;
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
    async fn retire_failed_transport(
        &self,
        id: &str,
        failed: &Arc<Mutex<transport::Transport>>,
    ) -> Result<()> {
        // Callers release the transport mutex before taking lifecycle ownership.
        // Connect/update/remove hold this same guard while replacing publication.
        let _lifecycle = self.connection_guard(id).await?;
        let removed = {
            let mut transports = self.transports.lock().await;
            if transports
                .get(id)
                .is_some_and(|current| Arc::ptr_eq(current, failed))
            {
                transports.remove(id)
            } else {
                None
            }
        };
        if let Some(removed) = removed {
            let mut registry = self.registry.lock().await;
            if let Some(connection) = registry.connections.iter_mut().find(|c| c.id == id) {
                connection.status = "disconnected".into();
                connection.last_error = Some("SSH bridge failed. Reconnect; inspect any submitted operation before retrying because its outcome may be unknown.".into());
            }
            drop(registry);
            removed.lock().await.close().await;
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
        if method == "run.create" {
            let expected = params.as_object_mut().unwrap().remove("expected_mode");
            ensure!(
                expected.as_ref().is_none_or(|mode| mode == &json!(c.mode)),
                "Crew connection privacy changed; refresh the verified workspace before granting agent access"
            );
        }
        if matches!(method, "message.post" | "blob.begin") {
            let mode = json!(c.mode);
            ensure!(
                params
                    .get("personal_mode")
                    .is_none_or(|expected| expected == &mode),
                "Crew connection privacy changed; refresh the verified workspace before sending"
            );
            params["personal_mode"] = mode;
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
        let transport = self.transport(id).await?;
        let mut locked = transport.lock().await;
        let result = self
            .signed_exchange(&mut locked, &c, method, params, request_id, &signer)
            .await;
        let usable = locked.is_usable();
        drop(locked);
        if !usable {
            self.retire_failed_transport(id, &transport).await?;
        }
        result
    }
    async fn signed_exchange(
        &self,
        t: &mut transport::Transport,
        c: &Connection,
        method: &str,
        params: Value,
        request_id: Option<String>,
        signer: &SigningKey,
    ) -> Result<Value> {
        let id = &c.id;
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
        self.validate_worker_scope(session, &s, &c).await?;
        Ok(
            json!({"connections":[{"id":c.id,"name":c.name,"status":c.status,"mode":c.mode,"workspace_id":c.workspace_id,"destination_channel_id":s.channel_id,"source_channel_ids":s.source_channels,"context_discovery":"Use context.manifest with empty params for recent authorized selected-channel context. Search each relevant source_channel_id with messages.search using channel_id and query; history and search are per-channel.","remote_files_enabled":!s.public_provider && c.remote_root.is_some(),"remote_execution_enabled":!s.public_provider && c.remote_root.is_some() && c.remote_execution,"remote_path_base":"the granted SSH work directory, not the local task directory; supply relative paths"}]}),
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
    pub async fn preflight_run(
        &self,
        id: &str,
        provider: &dyn Provider,
        policy: &RunPolicy,
    ) -> Result<Connection> {
        ensure!(!provider.uses_tool_bridge(), "Crew cannot admit providers with external tools outside its scoped capability boundary");
        let connection = self.connection(id).await?;
        ensure!(
            policy
                .expected_mode
                .is_none_or(|mode| mode == connection.mode),
            "Crew connection privacy changed; refresh the verified workspace before granting agent access"
        );
        let public = provider.tier() == ProviderTier::Public;
        ensure!(
            !public || connection.mode == ClusterMode::Public,
            "Private cluster blocks public models"
        );
        ensure!(
            !public || !policy.origin_restricted,
            "Private-origin local conversation cannot be admitted to a public Crew worker"
        );
        Ok(connection)
    }
    pub async fn begin_run(
        &self,
        session: &str,
        id: &str,
        channel: &str,
        sources: Vec<String>,
        provider: &dyn Provider,
    ) -> Result<RunAdmission> {
        self.begin_run_with_policy(
            session,
            id,
            channel,
            sources,
            provider,
            RunPolicy::default(),
        )
        .await
    }
    pub async fn begin_run_with_policy(
        &self,
        session: &str,
        id: &str,
        channel: &str,
        mut sources: Vec<String>,
        provider: &dyn Provider,
        policy: RunPolicy,
    ) -> Result<RunAdmission> {
        let mut origin_restricted = policy.origin_restricted;
        let c = self.preflight_run(id, provider, &policy).await?;
        let public = provider.tier() == ProviderTier::Public;
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
        let result=self.signed_request(id,"run.create",json!({"expected_mode":c.mode,"channel_id":channel,"source_channels":sources,"provider_policy_id":provider_binding(provider),"personal_mode":if origin_restricted {ClusterMode::Private}else{c.mode},"public_provider":public,"expires_in":3600,"remote_root":if public {None}else{c.remote_root.clone()},"remote_execution":!public && c.remote_execution}),None).await?;
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
                    source_channels: sources.clone(),
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
                "source_channel_ids": sources,
                "context_discovery": "The included history covers only the destination channel, not all selected context. Call context.manifest with empty params for recent authorized selected-channel context (up to 200 messages). For more targeted evidence, call messages.search with channel_id and query for each relevant source_channel_id. Do not assume this initial history contains the answer.",
                "history_channel_id": channel,
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
        self.begin_run_with_policy(
            session,
            id,
            channel,
            sources,
            provider,
            RunPolicy {
                origin_restricted,
                expected_mode: None,
            },
        )
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
        let mut locked = transport.lock().await;
        self.validate_worker_scope(session, &s, &c).await?;
        let credential = self.read_credential(&format!("run:{session}"))?;
        let result = locked
            .request(method, params, None, Some(&credential), None)
            .await;
        let usable = locked.is_usable();
        drop(locked);
        if !usable {
            self.retire_failed_transport(&s.connection_id, &transport)
                .await?;
        }
        let result = result?;
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
            ExtensionManager,
        },
        privacy::{CallCapability, ProviderTier},
        session::SessionManager,
    };
    use rmcp::model::CallToolResult;
    #[cfg(unix)]
    use std::time::Duration;
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
    async fn message_and_blob_mode_mismatches_refuse_before_credentials_or_transport() {
        let root = fixture_root("mode-guard");
        let manager = CrewManager::new(root.clone()).unwrap();
        let connection_id = "mode-guard-connection";
        manager.registry.lock().await.connections.push(Connection {
            id: connection_id.into(),
            node_id: None,
            name: "mode guard fixture".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id: "mode-guard-workspace".into(),
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: "mode-guard-cluster".into(),
            mode: ClusterMode::Public,
            policy_epoch: 1,
            status: "connected".into(),
            last_error: None,
            device_id: "22".repeat(32),
            public_key: "33".repeat(32),
        });

        for method in ["message.post", "blob.begin"] {
            let mismatch = manager
                .human_request(
                    connection_id,
                    method,
                    json!({"personal_mode":"private"}),
                    None,
                )
                .await
                .expect_err("a stale private mode must be refused before I/O");
            assert_eq!(
                mismatch.to_string(),
                "Crew connection privacy changed; refresh the verified workspace before sending"
            );

            let missing = manager
                .human_request(connection_id, method, json!({}), None)
                .await
                .expect_err("the fixture intentionally has no device credential");
            assert!(
                !missing.to_string().contains("privacy changed"),
                "omitted mode should remain backward-compatible: {missing}"
            );

            let matching = manager
                .human_request(
                    connection_id,
                    method,
                    json!({"personal_mode":"public"}),
                    None,
                )
                .await
                .expect_err("matching mode reaches the credential boundary in this fixture");
            assert!(
                !matching.to_string().contains("privacy changed"),
                "matching mode was rejected by the privacy guard: {matching}"
            );
        }

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn run_admission_mode_policy_refuses_before_transport_and_preserves_private_origin() {
        let root = fixture_root("run-mode-policy");
        let manager = CrewManager::new(root.clone()).unwrap();
        let connection_id = "run-mode-policy-connection";
        manager.registry.lock().await.connections.push(Connection {
            id: connection_id.into(),
            node_id: None,
            name: "run mode policy fixture".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id: "run-mode-policy-workspace".into(),
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: "run-mode-policy-cluster".into(),
            mode: ClusterMode::Public,
            policy_epoch: 1,
            status: "connected".into(),
            last_error: None,
            device_id: "22".repeat(32),
            public_key: "33".repeat(32),
        });
        let provider = crate::providers::testprovider::TestProvider::new_replaying(
            root.join("missing-cassette.json")
                .to_string_lossy()
                .into_owned(),
        )
        .unwrap();

        let private_connection_id = "run-mode-policy-private-connection";
        let mut private_connection = manager.registry.lock().await.connections[0].clone();
        private_connection.id = private_connection_id.into();
        private_connection.mode = ClusterMode::Private;
        manager
            .registry
            .lock()
            .await
            .connections
            .push(private_connection);
        let private_public = match manager
            .begin_run_with_policy(
                "run-mode-policy-private-cluster",
                private_connection_id,
                "destination-channel",
                vec![],
                &provider,
                RunPolicy::default(),
            )
            .await
        {
            Err(error) => error,
            Ok(_) => panic!(
                "a public provider must be refused by a private cluster before manager reservation"
            ),
        };
        assert_eq!(
            private_public.to_string(),
            "Private cluster blocks public models"
        );
        assert!(!manager
            .registry
            .lock()
            .await
            .scopes
            .contains_key("run-mode-policy-private-cluster"));

        let mismatch = manager
            .begin_run_with_policy(
                "run-mode-policy-session",
                connection_id,
                "destination-channel",
                vec![],
                &provider,
                RunPolicy {
                    origin_restricted: false,
                    expected_mode: Some(ClusterMode::Private),
                },
            )
            .await
            .err()
            .expect("a stale private mode must stop admission before signing");
        assert_eq!(
            mismatch.to_string(),
            "Crew connection privacy changed; refresh the verified workspace before granting agent access"
        );

        let legacy = manager
            .begin_run_with_policy(
                "run-mode-policy-legacy-session",
                connection_id,
                "destination-channel",
                vec![],
                &provider,
                RunPolicy::default(),
            )
            .await
            .err()
            .expect("the fixture intentionally has no device credential");
        assert!(
            !legacy.to_string().contains("privacy changed"),
            "missing expected_mode must preserve the legacy path: {legacy}"
        );

        let private_origin = manager
            .begin_run_with_policy(
                "run-mode-policy-private-origin",
                connection_id,
                "destination-channel",
                vec![],
                &provider,
                RunPolicy {
                    origin_restricted: true,
                    expected_mode: Some(ClusterMode::Public),
                },
            )
            .await
            .err()
            .expect("a private-origin run must not be admitted to a public provider");
        assert_eq!(
            private_origin.to_string(),
            "Private-origin local conversation cannot be admitted to a public Crew worker"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn authoritative_crew_context_omits_local_moim_context() -> anyhow::Result<()> {
        let working_dir = tempfile::tempdir()?;
        let canary = format!("crew-local-context-canary-{}", uuid::Uuid::new_v4());
        fs::write(working_dir.path().join("AGENTS.md"), &canary)?;
        let canary_file = format!("{canary}.txt");
        fs::write(working_dir.path().join(&canary_file), b"local-only")?;
        let path_text = working_dir.path().display().to_string();
        let session_id = format!("crew-context-{canary}");
        let manager = crate::crew::install_test_scope(&session_id, None).await;

        let extension_manager = ExtensionManager::new_without_provider(working_dir.path().into());
        let moim = extension_manager
            .collect_moim(&session_id, working_dir.path(), None)
            .await
            .expect("Crew scope should still provide its remote guidance");
        assert!(moim.contains("Crew scope: remote file paths"), "{moim}");
        assert!(
            !moim.contains(&path_text),
            "Crew MOIM leaked local path: {moim}"
        );
        assert!(
            !moim.contains(&canary),
            "Crew MOIM leaked local workspace context: {moim}"
        );

        crate::crew::remove_test_scope(&manager, &session_id).await;
        Ok(())
    }

    #[tokio::test]
    async fn ordinary_context_retains_local_moim_and_prompt_context() -> anyhow::Result<()> {
        let working_dir = tempfile::tempdir()?;
        let canary = format!("ordinary-local-context-canary-{}", uuid::Uuid::new_v4());
        fs::write(working_dir.path().join("AGENTS.md"), &canary)?;
        let canary_file = format!("{canary}.txt");
        fs::write(working_dir.path().join(&canary_file), b"local-only")?;
        let path_text = working_dir.path().display().to_string();
        let session_id = format!("ordinary-context-{canary}");

        let extension_manager = ExtensionManager::new_without_provider(working_dir.path().into());
        let moim = extension_manager
            .collect_moim(&session_id, working_dir.path(), None)
            .await
            .expect("ordinary sessions should receive local context");
        assert!(
            moim.contains(&path_text) && moim.contains(&canary_file),
            "ordinary MOIM lost local context: {moim}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn agent_connections_exposes_only_live_authorized_context_channels() -> anyhow::Result<()>
    {
        let root = fixture_root("agent-connections-context-scope");
        let connection = Connection {
            id: "authorized-connection".into(),
            node_id: None,
            name: "authorized".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id: "workspace-authorized".into(),
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: "cluster-authorized".into(),
            mode: ClusterMode::Public,
            policy_epoch: 7,
            status: "connected".into(),
            last_error: None,
            device_id: "22".repeat(32),
            public_key: "33".repeat(32),
        };
        let mut other = connection.clone();
        other.id = "other-connection".into();
        other.name = "other".into();
        let scope = Scope {
            connection_id: connection.id.clone(),
            run_id: "run-authorized".into(),
            channel_id: "destination-channel".into(),
            source_channels: vec![
                "source-a".into(),
                "source-b".into(),
                "destination-channel".into(),
            ],
            epoch: connection.policy_epoch,
            provider_binding: "public-test-provider".into(),
            public_provider: true,
            origin_restricted: false,
            expired: false,
        };
        let manager = CrewManager::new(root.clone())?;
        {
            let mut registry = manager.registry.lock().await;
            registry.connections = vec![connection.clone(), other];
            registry.scopes.insert("live-session".into(), scope.clone());
        }

        let discovery = manager.agent_connections("live-session").await?;
        let entry = &discovery["connections"][0];
        assert_eq!(entry["id"], "authorized-connection");
        assert_eq!(entry["destination_channel_id"], "destination-channel");
        assert_eq!(
            entry["source_channel_ids"],
            json!(["source-a", "source-b", "destination-channel"])
        );
        assert!(entry["context_discovery"]
            .as_str()
            .is_some_and(|guidance| guidance.contains("messages.search")));
        let serialized = serde_json::to_string(&discovery)?;
        assert!(!serialized.contains("other-connection"));

        manager
            .registry
            .lock()
            .await
            .scopes
            .get_mut("live-session")
            .unwrap()
            .expired = true;
        let expired = manager.agent_connections("live-session").await.unwrap_err();
        assert!(expired
            .to_string()
            .contains("grant or connection policy changed"));

        manager
            .registry
            .lock()
            .await
            .scopes
            .get_mut("live-session")
            .unwrap()
            .expired = false;
        manager.registry.lock().await.connections[0].policy_epoch += 1;
        let changed = manager.agent_connections("live-session").await.unwrap_err();
        assert!(changed
            .to_string()
            .contains("grant or connection policy changed"));

        let missing = manager
            .agent_connections("missing-session")
            .await
            .unwrap_err();
        assert!(missing.to_string().contains("no human-approved Crew run"));

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn admitted_context_keeps_history_on_destination_and_lists_selected_sources(
    ) -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        if !crate::test_sandbox::in_a_process_of_its_own() {
            return Ok(());
        }
        let root = fixture_root("admission-context-discovery");
        let profile_root = root.join("profile");
        let fake_bin = root.join("bin");
        fs::create_dir_all(&fake_bin)?;
        let workspace_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let log = root.join("requests.log");
        let ssh = format!(
            r#"#!/bin/sh
log='{}'
if [ "$1" = "-G" ]; then
  printf '%s\n' \
    'hostname 127.0.0.1' 'port 22' 'stricthostkeychecking yes' \
    'forwardagent no' 'forwardx11 no' 'permitlocalcommand no' \
    'clearallforwardings yes' 'nohostauthenticationforlocalhost no' \
    'tunnel no' 'forkafterauthentication no' \
    'gssapidelegatecredentials no' 'proxycommand none' \
    'controlmaster no' 'controlpersist no' 'controlpath none'
  exit 0
fi
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$log"
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  if printf '%s\n' "$line" | grep -q 'auth.challenge'; then
    printf '{{"id":"%s","result":{{"workspace_id":"{workspace_id}","nonce":"nonce"}}}}\n' "$id"
  elif printf '%s\n' "$line" | grep -q 'run.create'; then
    printf '{{"id":"%s","result":{{"run":{{"id":"run-admitted"}},"credential":"run-credential"}}}}\n' "$id"
  elif printf '%s\n' "$line" | grep -q 'messages.history'; then
    printf '{{"id":"%s","result":{{"messages":[{{"channel_id":"destination-channel","id":"destination-message","text":"destination-only"}}]}}}}\n' "$id"
  else
    printf '{{"id":"%s","result":{{"accepted_method":"fixture"}}}}\n' "$id"
  fi
done
"#,
            log.display()
        );
        let ssh_path = fake_bin.join("ssh");
        fs::write(&ssh_path, ssh)?;
        fs::set_permissions(&ssh_path, fs::Permissions::from_mode(0o700))?;
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
        let device_key = SigningKey::from_bytes(&[7; 32]);
        let connection = Connection {
            id: "admission-connection".into(),
            node_id: None,
            name: "admission fixture".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id: workspace_id.into(),
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: "admission-cluster".into(),
            mode: ClusterMode::Public,
            policy_epoch: 1,
            status: "connected".into(),
            last_error: None,
            device_id: hex(&Sha256::digest(device_key.verifying_key().to_bytes())),
            public_key: hex(&device_key.verifying_key().to_bytes()),
        };
        let manager = CrewManager::new(root.join("manager"))?;
        manager
            .registry
            .lock()
            .await
            .connections
            .push(connection.clone());
        manager.write_credential(
            &format!("device:{}", connection.id),
            &hex(&device_key.to_bytes()),
        )?;
        let control = manager.control_path(&connection.id)?;
        let transport = transport::Transport::connect(&connection, &control).await?;
        manager
            .transports
            .lock()
            .await
            .insert(connection.id.clone(), Arc::new(Mutex::new(transport)));

        let provider = crate::providers::testprovider::TestProvider::new_replaying(
            root.join("provider-cassette.json").to_string_lossy(),
        )?;
        let admission = manager
            .begin_run(
                "admission-session",
                &connection.id,
                "destination-channel",
                vec!["source-a".into(), "source-b".into()],
                &provider,
            )
            .await?;
        let context: Value = serde_json::from_str(&admission.context)?;
        assert_eq!(context["destination_channel_id"], "destination-channel");
        assert_eq!(context["history_channel_id"], "destination-channel");
        assert_eq!(
            context["source_channel_ids"],
            json!(["source-a", "source-b", "destination-channel"])
        );
        let history = context["history"]["messages"].as_array().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0]["channel_id"], "destination-channel");
        assert_eq!(history[0]["text"], "destination-only");
        let requests = fs::read_to_string(&log)?;
        let history_request = requests
            .lines()
            .find(|line| line.contains("messages.history"))
            .expect("admission should fetch destination history");
        assert!(history_request.contains("destination-channel"));
        assert!(!history_request.contains("source-a"));
        assert!(!history_request.contains("source-b"));

        let discovery = manager.agent_connections("admission-session").await?;
        assert_eq!(
            discovery["connections"][0]["source_channel_ids"],
            json!(["source-a", "source-b", "destination-channel"])
        );
        assert!(discovery["connections"][0]["context_discovery"]
            .as_str()
            .unwrap()
            .contains("messages.search"));

        manager.disconnect(&connection.id).await?;
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[cfg(unix)]
    fn worker_race_connection(
        connection_id: &str,
        mode: ClusterMode,
        policy_epoch: u64,
        public_provider: bool,
    ) -> (Connection, Scope) {
        let connection = Connection {
            id: connection_id.into(),
            node_id: None,
            name: "worker-race".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(),
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into(),
            mode,
            policy_epoch,
            status: "connected".into(),
            last_error: None,
            device_id: "22".repeat(32),
            public_key: "33".repeat(32),
        };
        let scope = Scope {
            connection_id: connection_id.into(),
            run_id: "worker-race-run".into(),
            channel_id: "worker-race-channel".into(),
            source_channels: vec!["worker-race-channel".into()],
            epoch: policy_epoch,
            provider_binding: "worker-race-provider".into(),
            public_provider,
            origin_restricted: false,
            expired: false,
        };
        (connection, scope)
    }

    #[cfg(unix)]
    fn write_worker_race_ssh(root: &Path, wait_for_release: bool) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let fake_bin = root.join("bin");
        fs::create_dir_all(&fake_bin).unwrap();
        let log = root.join("requests.log");
        let gate = root.join("response-held");
        let release = root.join("release-response");
        if wait_for_release {
            fs::write(&gate, b"hold").unwrap();
        }
        let ssh = format!(
            r#"#!/bin/sh
log='{}'
gate='{}'
release='{}'
if [ "$1" = "-G" ]; then
  printf '%s\n' \
    'hostname 127.0.0.1' 'port 22' 'stricthostkeychecking yes' \
    'forwardagent no' 'forwardx11 no' 'permitlocalcommand no' \
    'clearallforwardings yes' 'nohostauthenticationforlocalhost no' \
    'tunnel no' 'forkafterauthentication no' \
    'gssapidelegatecredentials no' 'proxycommand none' \
    'controlmaster no' 'controlpersist no' 'controlpath none'
  exit 0
fi
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$log"
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  while [ -f "$gate" ] && [ ! -f "$release" ]; do sleep 0.01; done
  printf '{{"id":"%s","result":{{"accepted_method":"messages.history"}}}}\n' "$id"
done
"#,
            log.display(),
            gate.display(),
            release.display(),
        );
        let ssh_path = fake_bin.join("ssh");
        fs::write(&ssh_path, ssh).unwrap();
        fs::set_permissions(&ssh_path, fs::Permissions::from_mode(0o700)).unwrap();
        (log, release)
    }

    #[cfg(unix)]
    async fn worker_race_manager(
        root: &Path,
        connection: &Connection,
        scope: &Scope,
    ) -> Arc<CrewManager> {
        let registry = Registry {
            connections: vec![connection.clone()],
            scopes: HashMap::from([("worker-race-session".into(), scope.clone())]),
            pending_device: None,
            completed_preparations: HashMap::new(),
        };
        fs::write(
            root.join("connections.json"),
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        let manager = Arc::new(CrewManager::new(root.to_owned()).unwrap());
        manager
            .write_credential("run:worker-race-session", "run-credential")
            .unwrap();
        let control = manager.control_path(&connection.id).unwrap();
        let transport = transport::Transport::connect(connection, &control)
            .await
            .unwrap();
        manager
            .transports
            .lock()
            .await
            .insert(connection.id.clone(), Arc::new(Mutex::new(transport)));
        manager
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn worker_request_rechecks_policy_before_writing_transport() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("worker-race-before-write");
        let fake_bin = root.join("bin");
        let (log, _) = write_worker_race_ssh(&root, false);
        let profile_root = root.join("profile");
        fs::create_dir_all(&profile_root).unwrap();
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
        let connection_id = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
        let (connection, scope) =
            worker_race_connection(connection_id, ClusterMode::Public, 1, true);
        let manager = worker_race_manager(&root, &connection, &scope).await;
        let transport = manager
            .transports
            .lock()
            .await
            .get(connection_id)
            .cloned()
            .unwrap();
        let control = manager
            .worker_request(
                "worker-race-session",
                "messages.history",
                json!({
                    "channel_id": "worker-race-channel",
                    "limit": 1,
                }),
            )
            .await
            .unwrap();
        assert_eq!(control["accepted_method"], "messages.history");
        let baseline_requests = fs::read_to_string(&log).unwrap_or_default();
        assert_eq!(baseline_requests.lines().count(), 1);
        let held = transport.lock().await;
        let manager_for_worker = manager.clone();
        let worker = tokio::spawn(async move {
            manager_for_worker
                .worker_request(
                    "worker-race-session",
                    "messages.history",
                    json!({
                        "channel_id": "worker-race-channel",
                        "limit": 1,
                    }),
                )
                .await
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while tokio::time::Instant::now() < deadline && Arc::strong_count(&transport) < 3 {
            tokio::task::yield_now().await;
        }
        assert!(Arc::strong_count(&transport) >= 3);
        assert_eq!(
            fs::read_to_string(&log).unwrap_or_default(),
            baseline_requests
        );
        {
            let mut registry = manager.registry.lock().await;
            let current = registry
                .connections
                .iter_mut()
                .find(|candidate| candidate.id == connection_id)
                .unwrap();
            current.mode = ClusterMode::Private;
            current.policy_epoch = 2;
        }
        drop(held);
        let error = worker.await.unwrap().unwrap_err().to_string();
        assert!(error.contains("policy changed"), "{error}");
        assert_eq!(
            fs::read_to_string(&log).unwrap_or_default(),
            baseline_requests
        );
        let _ = manager.transports.lock().await.remove(connection_id);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn worker_request_reports_possible_effects_after_inflight_policy_change() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("worker-race-inflight-response");
        let fake_bin = root.join("bin");
        let (log, release) = write_worker_race_ssh(&root, true);
        let profile_root = root.join("profile");
        fs::create_dir_all(&profile_root).unwrap();
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
        let connection_id = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
        let (connection, scope) =
            worker_race_connection(connection_id, ClusterMode::Private, 1, false);
        let manager = worker_race_manager(&root, &connection, &scope).await;
        let manager_for_worker = manager.clone();
        let worker = tokio::spawn(async move {
            manager_for_worker
                .worker_request(
                    "worker-race-session",
                    "messages.history",
                    json!({
                        "channel_id": "worker-race-channel",
                        "limit": 1,
                    }),
                )
                .await
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < deadline
            && fs::read_to_string(&log).unwrap_or_default().is_empty()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!fs::read_to_string(&log).unwrap_or_default().is_empty());
        {
            let mut registry = manager.registry.lock().await;
            let current = registry
                .connections
                .iter_mut()
                .find(|candidate| candidate.id == connection_id)
                .unwrap();
            current.policy_epoch = 2;
        }
        fs::write(&release, b"release").unwrap();
        let error = tokio::time::timeout(Duration::from_secs(2), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("inspect any submitted effects before obtaining a fresh grant"),
            "{error}"
        );
        let _ = manager.transports.lock().await.remove(connection_id);
        let _ = fs::remove_dir_all(root);
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
if [ "$1" = "-G" ]; then
  printf '%s\n' \
    'hostname 127.0.0.1' 'port 22' 'stricthostkeychecking yes' \
    'forwardagent no' 'forwardx11 no' 'permitlocalcommand no' \
    'clearallforwardings yes' 'nohostauthenticationforlocalhost no' \
    'tunnel no' 'forkafterauthentication no' \
    'gssapidelegatecredentials no' 'proxycommand none' \
    'controlmaster no' 'controlpersist no' 'controlpath none'
  exit 0
fi
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
