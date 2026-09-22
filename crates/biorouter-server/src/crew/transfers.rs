use super::local_files::{self, Direction, Selection, CHUNK, MAX_SIZE};
use anyhow::{ensure, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

static SERVICES: LazyLock<Mutex<BTreeMap<PathBuf, Arc<TransferService>>>> =
    LazyLock::new(Mutex::default);
const MAX_RECEIPTS: usize = 32;
fn id() -> String {
    hex::encode(rand::random::<[u8; 16]>())
}
fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FilePurpose {
    #[default]
    Transfer,
    Cleanup,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FileRequest {
    #[serde(default)]
    pub expected_mode: Option<biorouter::crew::ClusterMode>,
    #[serde(default)]
    pub purpose: FilePurpose,
    pub connection_id: String,
    pub channel_id: String,
    pub direction: Direction,
    pub path: PathBuf,
    #[serde(default)]
    pub overwrite: bool,
    pub blob_id: Option<String>,
    pub transfer_id: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    pub request_id: String,
    pub connection_id: String,
    pub channel_id: String,
    pub direction: Direction,
    pub file_capability: String,
    pub blob_id: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Receipt {
    pub id: String,
    pub request_id: String,
    pub connection_id: String,
    pub channel_id: String,
    pub direction: Direction,
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub offset: u64,
    pub blob_id: Option<String>,
    pub state: String,
    pub error: Option<String>,
    binding: String,
    #[serde(default)]
    intent: String,
    #[serde(default)]
    local_selection: String,
    #[serde(default)]
    destination_identity: Option<String>,
}
struct Capability {
    purpose: FilePurpose,
    selection: Selection,
    connection_id: String,
    channel_id: String,
    blob_id: Option<String>,
    transfer_id: Option<String>,
    expires: Instant,
    binding: String,
    request_id: Option<String>,
    replay_receipt_id: Option<String>,
}
#[derive(Default)]
struct State {
    receipts: BTreeMap<String, Receipt>,
    capabilities: BTreeMap<String, Capability>,
    active: BTreeMap<String, CancellationToken>,
    poisoned: bool,
}
pub struct TransferService {
    directory: local_files::ProtectedDirectory,
    _lock: std::fs::File,
    state: Mutex<State>,
    slots: Arc<Semaphore>,
    selection_slots: Semaphore,
}

pub async fn service() -> Result<Arc<TransferService>> {
    let path = biorouter::config::paths::Paths::state_dir().join("crew/transfers");
    let mut services = SERVICES.lock().await;
    if let Some(service) = services.get(&path) {
        return Ok(service.clone());
    }
    let service = Arc::new(TransferService::open(&path)?);
    services.insert(path, service.clone());
    Ok(service)
}

impl TransferService {
    fn open(path: &Path) -> Result<Self> {
        let directory =
            local_files::ProtectedDirectory::new(local_files::open_directory(path, true)?)?;
        local_files::protected_directory(&directory)?;
        #[cfg(unix)]
        {
            use cap_std::fs::MetadataExt;
            let metadata = directory.dir_metadata()?;
            ensure!(
                metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
                "Crew transfer store must be private to this OS account"
            );
        }
        #[cfg(windows)]
        local_files::windows::private_directory(&directory)?;
        let lock = directory
            .open_with(
                "writer.lock",
                local_files::nofollow_options()
                    .read(true)
                    .write(true)
                    .create(true),
            )?
            .into_std();
        #[cfg(windows)]
        local_files::windows::validate_partial(&lock)?;
        lock.try_lock_exclusive()
            .context("Another daemon owns the transfer receipts")?;
        let mut receipts: BTreeMap<String, Receipt> = match directory
            .open_with("receipts.json", local_files::nofollow_options().read(true))
        {
            Ok(file) => {
                let file = file.into_std();
                #[cfg(windows)]
                local_files::windows::validate_partial(&file)?;
                serde_json::from_reader(file.take(1024 * 1024))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            receipts.len() <= MAX_RECEIPTS,
            "Crew transfer receipt quota exceeded"
        );
        for (key, receipt) in &mut receipts {
            ensure!(
                key == &receipt.id
                    && key.len() == 32
                    && hex::decode(key).is_ok()
                    && receipt.size <= MAX_SIZE
                    && receipt.offset <= receipt.size,
                "Invalid transfer receipt"
            );
            if matches!(
                receipt.state.as_str(),
                "publishing" | "publication_unconfirmed"
            ) {
                receipt.state = "publication_unconfirmed".into();
            } else if receipt.state != "completed" {
                receipt.state = "needs_file_selection".into();
            }
        }
        Ok(Self {
            directory,
            _lock: lock,
            state: Mutex::new(State {
                receipts,
                ..State::default()
            }),
            slots: Arc::new(Semaphore::new(2)),
            selection_slots: Semaphore::new(4),
        })
    }
    fn persist(&self, state: &mut State) -> Result<()> {
        ensure!(
            !state.poisoned,
            "Transfer receipt storage failed; restart the daemon before retrying"
        );
        let temporary = format!(".receipts-{}.tmp", id());
        let bytes = serde_json::to_vec(&state.receipts)?;
        state.poisoned = true;
        self.directory.revalidate()?;
        let mut file = self
            .directory
            .open_with(
                &temporary,
                local_files::nofollow_options().write(true).create_new(true),
            )?
            .into_std();
        #[cfg(windows)]
        local_files::windows::validate_partial(&file)?;
        file.write_all(&bytes)?;
        self.directory
            .publish_file(&file, &temporary, "receipts.json", true)?;
        state.poisoned = false;
        Ok(())
    }
    pub async fn register(&self, request: FileRequest) -> Result<Value> {
        let _selection = self
            .selection_slots
            .try_acquire()
            .context("Too many file selections are pending")?;
        ensure!(
            request.connection_id.len() <= 128 && request.channel_id.len() <= 128,
            "Invalid transfer scope"
        );
        ensure!(
            request.purpose != FilePurpose::Cleanup
                || (request.direction == Direction::Download
                    && request
                        .transfer_id
                        .as_ref()
                        .is_some_and(|id| !id.is_empty())
                    && !request.overwrite),
            "Cleanup approval requires a download transfer_id and cannot authorize replacement"
        );
        let binding = self.selection_binding(&request).await?;
        let replay_receipt_id = self.registration_replay(&request, &binding).await?;
        let replay_download =
            replay_receipt_id.is_some() && request.direction == Direction::Download;
        let selection = tokio::task::spawn_blocking(move || {
            let selected = if request.purpose == FilePurpose::Cleanup {
                local_files::select_cleanup(&request.path)?
            } else if replay_download {
                local_files::select_download_replay(&request.path, request.overwrite)?
            } else {
                local_files::select(&request.path, request.direction, request.overwrite)?
            };
            Ok::<_, anyhow::Error>((request, selected))
        })
        .await??;
        let (request, selection) = selection;
        let mut state = self.state.lock().await;
        state
            .capabilities
            .retain(|_, capability| capability.expires > Instant::now());
        ensure!(
            state.capabilities.len() < 32,
            "Too many pending file selections"
        );
        let capability_id = id();
        let result =
            json!({"capability_id":capability_id,"name":selection.name(),"size":selection.size()});
        state.capabilities.insert(
            capability_id,
            Capability {
                purpose: request.purpose,
                selection,
                connection_id: request.connection_id,
                channel_id: request.channel_id,
                blob_id: request.blob_id,
                transfer_id: request.transfer_id,
                expires: Instant::now() + Duration::from_secs(300),
                binding,
                request_id: request.request_id,
                replay_receipt_id,
            },
        );
        Ok(result)
    }
    async fn registration_replay(
        &self,
        request: &FileRequest,
        binding: &str,
    ) -> Result<Option<String>> {
        let Some(request_id) = &request.request_id else {
            return Ok(None);
        };
        ensure!(
            !request_id.is_empty()
                && request_id.len() <= 128
                && request.purpose == FilePurpose::Transfer
                && request.transfer_id.is_none(),
            "A replay request_id must identify a transfer start, not resume or cleanup"
        );
        let state = self.state.lock().await;
        let Some(receipt) = state
            .receipts
            .values()
            .find(|receipt| receipt.request_id == *request_id)
        else {
            return Ok(None);
        };
        ensure!(
            receipt.connection_id == request.connection_id
                && receipt.channel_id == request.channel_id
                && receipt.direction == request.direction
                && (request.direction == Direction::Upload || receipt.blob_id == request.blob_id)
                && receipt.binding == binding,
            "Idempotency key belongs to a different transfer or connection policy"
        );
        Ok(Some(receipt.id.clone()))
    }
    async fn selection_binding(&self, request: &FileRequest) -> Result<String> {
        if request.purpose == FilePurpose::Transfer {
            return connection_binding_with_expected(&request.connection_id, request.expected_mode)
                .await;
        }
        let state = self.state.lock().await;
        let receipt = request
            .transfer_id
            .as_ref()
            .and_then(|id| state.receipts.get(id))
            .context("Unknown cleanup transfer")?;
        ensure!(
            receipt.direction == Direction::Download
                && receipt.connection_id == request.connection_id
                && receipt.channel_id == request.channel_id
                && receipt.blob_id == request.blob_id,
            "Cleanup approval does not match the saved transfer"
        );
        Ok(receipt.binding.clone())
    }
    pub async fn list(&self) -> Vec<Receipt> {
        self.state.lock().await.receipts.values().cloned().collect()
    }
    pub async fn get(&self, id: &str) -> Result<Receipt> {
        self.state
            .lock()
            .await
            .receipts
            .get(id)
            .cloned()
            .context("Unknown transfer")
    }
    fn take_file(
        state: &mut State,
        capability: &str,
        receipt: &Receipt,
        resuming: bool,
        purpose: FilePurpose,
    ) -> Result<Selection> {
        let cap = state
            .capabilities
            .remove(capability)
            .context("Select the local file again")?;
        ensure!(
            cap.purpose == purpose
                && cap.binding == receipt.binding
                && cap.expires > Instant::now()
                && cap.connection_id == receipt.connection_id
                && cap.channel_id == receipt.channel_id
                && cap.selection.direction() == receipt.direction
                && cap
                    .request_id
                    .as_deref()
                    .is_none_or(|id| id == receipt.request_id)
                && cap
                    .replay_receipt_id
                    .as_deref()
                    .is_none_or(|id| !resuming && id == receipt.id)
                && (receipt.direction == Direction::Upload || cap.blob_id == receipt.blob_id)
                && if resuming {
                    cap.transfer_id.as_deref() == Some(receipt.id.as_str())
                } else {
                    cap.transfer_id.is_none()
                },
            "Local file approval does not match this transfer"
        );
        Ok(cap.selection)
    }
    pub async fn start(self: &Arc<Self>, request: StartRequest) -> Result<Receipt> {
        ensure!(
            !request.request_id.is_empty() && request.request_id.len() <= 128,
            "A bounded request_id is required"
        );
        let binding = connection_binding(&request.connection_id).await?;
        let mut state = self.state.lock().await;
        ensure!(!state.poisoned, "Transfer store needs recovery");
        let previous = state
            .receipts
            .values()
            .find(|r| r.request_id == request.request_id)
            .cloned();
        let mut receipt = Receipt {
            id: previous
                .as_ref()
                .map(|receipt| receipt.id.clone())
                .unwrap_or_else(id),
            request_id: request.request_id,
            connection_id: request.connection_id,
            channel_id: request.channel_id,
            direction: request.direction,
            name: String::new(),
            size: 0,
            sha256: String::new(),
            offset: 0,
            blob_id: request.blob_id,
            state: "starting".into(),
            error: None,
            binding,
            intent: String::new(),
            local_selection: String::new(),
            destination_identity: None,
        };
        let selection = Self::take_file(
            &mut state,
            &request.file_capability,
            &receipt,
            false,
            FilePurpose::Transfer,
        )?;
        receipt.local_selection = local_files::selection_identity(&selection)?;
        receipt.intent = digest(&serde_json::to_vec(&json!([
            "crew-transfer-intent-v2",
            receipt.connection_id,
            receipt.channel_id,
            receipt.direction,
            receipt.blob_id,
            receipt.binding,
            receipt.local_selection
        ]))?);
        if let Some(previous) = previous {
            ensure!(
                !previous.local_selection.is_empty()
                    && previous.local_selection == receipt.local_selection
                    && previous.intent == receipt.intent,
                "Idempotency key belongs to a different file selection or transfer; inspect the existing receipt before starting a new request"
            );
            return Ok(previous);
        }
        ensure!(
            state.receipts.len() < MAX_RECEIPTS,
            "32 transfers retained; forget a receipt before starting another"
        );
        receipt.name = selection.name().into();
        receipt.size = selection.size().unwrap_or(0);
        state.receipts.insert(receipt.id.clone(), receipt.clone());
        self.launch(&mut state, receipt, selection)
    }
    pub async fn resume(self: &Arc<Self>, id: &str, capability: &str) -> Result<Receipt> {
        let mut state = self.state.lock().await;
        ensure!(
            !state.poisoned && !state.active.contains_key(id),
            "Transfer is active or storage needs recovery"
        );
        let receipt = state
            .receipts
            .get(id)
            .cloned()
            .context("Unknown transfer")?;
        ensure!(!matches!(receipt.state.as_str(), "completed" | "publishing" | "publication_unconfirmed"),
            "Transfer is completed or publication outcome needs inspection; do not replay publication");
        let selection = Self::take_file(
            &mut state,
            capability,
            &receipt,
            true,
            FilePurpose::Transfer,
        )?;
        self.launch(&mut state, receipt, selection)
    }
    pub async fn pause(&self, id: &str) -> Result<Receipt> {
        let mut state = self.state.lock().await;
        let Some(cancel) = state.active.get(id) else {
            return state.receipts.get(id).cloned().context("Unknown transfer");
        };
        cancel.cancel();
        let receipt = state.receipts.get_mut(id).context("Unknown transfer")?;
        if receipt.state != "completed" {
            receipt.state = "pause_requested".into();
        }
        let result = receipt.clone();
        self.persist(&mut state)?;
        Ok(result)
    }
    pub async fn forget(&self, id: &str, capability: Option<&str>) -> Result<()> {
        let mut state = self.state.lock().await;
        ensure!(!state.poisoned, "Transfer store needs recovery");
        ensure!(
            !state.active.contains_key(id),
            "Pause and wait for the transfer before forgetting its receipt"
        );
        let receipt = state
            .receipts
            .get(id)
            .cloned()
            .context("Unknown transfer")?;
        if receipt.direction == Direction::Download
            && receipt.state != "completed"
            && receipt.destination_identity.is_some()
        {
            let capability = capability.context("Reselect the original destination to remove its partial before forgetting this receipt")?;
            let Selection::Destination {
                directory, name, ..
            } = Self::take_file(&mut state, capability, &receipt, true, FilePurpose::Cleanup)?
            else {
                anyhow::bail!("A matching destination selection is required");
            };
            ensure!(
                Some(local_files::destination_identity(&directory, &name)?)
                    == receipt.destination_identity,
                "Reselect the original download destination"
            );
            directory.revalidate()?;
            let part = local_files::part_name(id);
            match directory.open_with(&part, local_files::nofollow_options().read(true)) {
                Ok(file) => {
                    validate_cleanup_partial(&file.into_std(), receipt.size)?;
                    directory.remove_file(part)?;
                    local_files::namespace_checkpoint(&directory)?.accept_receipt_reload_recovery();
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        state.receipts.remove(id);
        self.persist(&mut state)
    }
    async fn save(&self, receipt: &Receipt, cancel: &CancellationToken) -> Result<()> {
        let mut state = self.state.lock().await;
        ensure!(!cancel.is_cancelled(), "Transfer paused");
        if receipt.direction == Direction::Download && receipt.state != "completed" {
            let reserved: u64 = state
                .receipts
                .values()
                .filter(|other| {
                    other.id != receipt.id
                        && other.direction == Direction::Download
                        && other.state != "completed"
                })
                .map(|other| other.size)
                .sum();
            ensure!(reserved + receipt.size <= 4 * MAX_SIZE,
                "Pending downloads reserve 4 GiB; finish or remove an existing partial transfer first");
        }
        state.receipts.insert(receipt.id.clone(), receipt.clone());
        self.persist(&mut state)
    }
    fn launch(
        self: &Arc<Self>,
        state: &mut State,
        mut receipt: Receipt,
        selection: Selection,
    ) -> Result<Receipt> {
        ensure!(!state.poisoned, "Transfer store needs recovery");
        ensure!(
            !state.active.contains_key(&receipt.id),
            "Transfer already active"
        );
        let permit = match self.slots.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                if let Some(saved) = state.receipts.get_mut(&receipt.id) {
                    saved.state = "needs_file_selection".into();
                    saved.error = Some(
                        "Two transfers are active; reselect and resume when one finishes".into(),
                    );
                }
                self.persist(state)?;
                anyhow::bail!("Two transfers are active; resume this receipt when one finishes");
            }
        };
        let cancel = CancellationToken::new();
        receipt.state = "starting".into();
        receipt.error = None;
        state.active.insert(receipt.id.clone(), cancel.clone());
        state.receipts.insert(receipt.id.clone(), receipt.clone());
        if let Err(error) = self.persist(state) {
            state.active.remove(&receipt.id);
            return Err(error);
        }
        let accepted = receipt.clone();
        let service = self.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let mut receipt = receipt;
            let outcome = service.run(&mut receipt, selection, &cancel).await;
            let mut state = service.state.lock().await;
            state.active.remove(&receipt.id);
            if let Err(error) = outcome {
                tracing::error!("Crew transfer {} failed: {error}", receipt.id);
                if receipt.direction == Direction::Download
                    && matches!(receipt.state.as_str(), "publishing" | "completed")
                {
                    receipt.state = "publication_unconfirmed".into();
                } else {
                    receipt.state = "needs_file_selection".into();
                }
                receipt.error = Some(
                    transfer_recovery_message(&error, receipt.state == "publication_unconfirmed")
                        .into(),
                );
                state.receipts.insert(receipt.id.clone(), receipt);
                let _ = service.persist(&mut state);
            }
        });
        Ok(accepted)
    }
}

#[cfg(all(test, unix))]
#[path = "transfers_tests.rs"]
mod transfers_tests;

fn transfer_recovery_message(error: &anyhow::Error, publication_unconfirmed: bool) -> &'static str {
    if publication_unconfirmed {
        return "Publication could not be confirmed. Inspect the destination file before retrying this transfer.";
    }
    for cause in error.chain() {
        match cause.to_string().as_str() {
            "Crew credential vault is locked; explicitly unlock it for this daemon session" => {
                return "Unlock the Crew credential vault for this daemon session, then reselect the original local file or destination and resume.";
            }
            "Crew connection is disconnected; authenticate and connect in Crew" => {
                return "Authenticate and reconnect the saved connection in Crew, then reselect the original local file or destination and resume.";
            }
            "Connection identity or privacy policy changed; create a new approved transfer"
            | "Connection changed during transfer; inspect remote effects before retrying"
            | "Crew connection policy changed while this action was queued; review and retry"
            | "Crew connection policy changed during authentication; review and retry"
            | "Connection changed while authentication was pending; reconnect" => {
                return "The Crew connection or privacy policy changed. Review the connection and inspect any remote effects before starting a new approved transfer.";
            }
            "Transfer paused" => {
                return "Transfer paused. Reselect the original local file or destination to resume.";
            }
            _ => {}
        }
    }
    "Transfer stopped. Reselect the original local file or destination to resume. Inspect any unconfirmed publication before retrying."
}

async fn connection_binding(id: &str) -> Result<String> {
    connection_binding_with_expected(id, None).await
}
async fn connection_binding_with_expected(
    id: &str,
    expected_mode: Option<biorouter::crew::ClusterMode>,
) -> Result<String> {
    let connection = biorouter::crew::manager()?.connection(id).await?;
    ensure!(
        expected_mode.is_none_or(|mode| mode == connection.mode),
        "Crew connection privacy changed; refresh the verified workspace before selecting a file"
    );
    binding_for_connection(&connection)
}
fn binding_for_connection(connection: &biorouter::crew::Connection) -> Result<String> {
    Ok(digest(&serde_json::to_vec(&json!([
        connection.workspace_id,
        connection.workspace_public_key,
        connection.device_id,
        connection.mode,
        connection.policy_epoch,
        connection.owner_uid,
    ]))?))
}
async fn remote(receipt: &Receipt, method: &str, mut params: Value) -> Result<Value> {
    let connection = biorouter::crew::manager()?
        .connection(&receipt.connection_id)
        .await?;
    ensure!(
        binding_for_connection(&connection)? == receipt.binding,
        "Connection identity or privacy policy changed; create a new approved transfer"
    );
    if method == "blob.begin" {
        ensure!(params.is_object(), "Blob parameters must be an object");
        params["personal_mode"] = json!(connection.mode);
    }
    let result = biorouter::crew::manager()?
        .human_request(&receipt.connection_id, method, params, None)
        .await?;
    ensure!(
        connection_binding(&receipt.connection_id).await? == receipt.binding,
        "Connection changed during transfer; inspect remote effects before retrying"
    );
    Ok(result)
}
#[derive(Deserialize)]
struct Blob {
    id: String,
    channel_id: String,
    size: u64,
    sha256: String,
    offset: u64,
    complete: bool,
    #[serde(default)]
    media_type: String,
}
impl Blob {
    fn validate(&self, receipt: &Receipt) -> Result<()> {
        ensure!(
            self.size <= MAX_SIZE
                && self.offset <= self.size
                && self.channel_id == receipt.channel_id,
            "Remote attachment metadata differs from the approved transfer"
        );
        if let Some(id) = &receipt.blob_id {
            ensure!(&self.id == id, "Remote attachment identity changed");
        }
        if !receipt.sha256.is_empty() {
            ensure!(
                self.size == receipt.size && self.sha256 == receipt.sha256,
                "Remote attachment digest or size changed"
            );
        }
        ensure!(
            self.sha256.len() == 64 && hex::decode(&self.sha256)?.len() == 32,
            "Invalid remote digest"
        );
        Ok(())
    }
}

impl TransferService {
    async fn run(
        &self,
        receipt: &mut Receipt,
        selection: Selection,
        cancel: &CancellationToken,
    ) -> Result<()> {
        match selection {
            Selection::Source { file, stamp, .. } => {
                self.upload(receipt, file, stamp, cancel).await
            }
            Selection::Destination {
                directory,
                name,
                overwrite,
            } => {
                self.download(receipt, directory, name, overwrite, cancel)
                    .await
            }
        }
    }
    async fn upload(
        &self,
        receipt: &mut Receipt,
        mut file: std::fs::File,
        stamp: local_files::Stamp,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let size = stamp.size();
        let (source, sha256) = tokio::task::spawn_blocking(move || {
            stamp.unchanged(&file)?;
            let sha = local_files::hash(&mut file, size)?;
            stamp.unchanged(&file)?;
            Ok::<_, anyhow::Error>(((file, stamp), sha))
        })
        .await??;
        let (mut file, stamp) = source;
        file.seek(SeekFrom::Start(0))?;
        let mut header = [0u8; 16];
        let header_size = file.read(&mut header)?;
        let media_type = image_type(&header[..header_size]).unwrap_or("application/octet-stream");
        ensure!(
            receipt.sha256.is_empty() || (receipt.sha256 == sha256 && receipt.size == size),
            "Reselected file differs from the original transfer"
        );
        receipt.size = size;
        receipt.sha256 = sha256;
        receipt.state = "uploading".into();
        receipt.error = None;
        self.save(receipt, cancel).await?;
        let mut blob: Blob = serde_json::from_value(if let Some(blob_id) = &receipt.blob_id {
            remote(receipt, "blob.status", json!({"blob_id":blob_id})).await?
        } else {
            remote(
                receipt,
                "blob.begin",
                json!({"channel_id":receipt.channel_id,"name":receipt.name,
                "media_type":media_type,"size":size,"sha256":receipt.sha256,
                "idempotency_key":format!("{}-begin",receipt.id)}),
            )
            .await?
        })?;
        blob.validate(receipt)?;
        receipt.blob_id = Some(blob.id.clone());
        receipt.offset = blob.offset;
        self.save(receipt, cancel).await?;
        let mut buffer = vec![0; CHUNK];
        while blob.offset < size {
            ensure!(!cancel.is_cancelled(), "Transfer paused");
            stamp.unchanged(&file)?;
            file.seek(SeekFrom::Start(blob.offset))?;
            let count = (size - blob.offset).min(CHUNK as u64) as usize;
            file.read_exact(&mut buffer[..count])?;
            let old_offset = blob.offset;
            blob = serde_json::from_value(remote(receipt, "blob.chunk", json!({
                "blob_id":blob.id,"offset":old_offset,"data_hex":hex::encode(&buffer[..count]),
                "idempotency_key":format!("{}-{}-{}",receipt.id,old_offset,digest(&buffer[..count]))
            })).await?)?;
            blob.validate(receipt)?;
            ensure!(
                blob.offset == old_offset + count as u64,
                "Remote upload offset did not match the submitted chunk"
            );
            receipt.offset = blob.offset;
            self.save(receipt, cancel).await?;
        }
        stamp.unchanged(&file)?;
        ensure!(!cancel.is_cancelled(), "Transfer paused");
        if !blob.complete {
            blob = serde_json::from_value(
                remote(
                    receipt,
                    "blob.finish",
                    json!({
                        "blob_id":blob.id,"idempotency_key":format!("{}-finish",receipt.id)
                    }),
                )
                .await?,
            )?;
        }
        blob.validate(receipt)?;
        ensure!(
            blob.complete && blob.offset == size,
            "Remote attachment commit is not confirmed"
        );
        receipt.state = "completed".into();
        self.save(receipt, cancel).await
    }
    async fn download(
        &self,
        receipt: &mut Receipt,
        directory: local_files::ProtectedDirectory,
        name: String,
        overwrite: bool,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let blob_id = receipt
            .blob_id
            .clone()
            .context("Download requires a blob_id")?;
        let blob: Blob = serde_json::from_value(
            remote(receipt, "blob.status", json!({"blob_id":blob_id})).await?,
        )?;
        blob.validate(receipt)?;
        ensure!(blob.complete, "Attachment has not been committed");
        receipt.size = blob.size;
        receipt.sha256 = blob.sha256;
        receipt.state = "downloading".into();
        receipt.error = None;
        let destination_identity = local_files::destination_identity(&directory, &name)?;
        ensure!(
            receipt
                .destination_identity
                .as_ref()
                .is_none_or(|original| original == &destination_identity),
            "Reselect the original download destination; its directory or filename differs"
        );
        receipt.destination_identity = Some(destination_identity);
        self.save(receipt, cancel).await?;
        directory.revalidate()?;
        let part = local_files::part_name(&receipt.id);
        let mut options = local_files::nofollow_options();
        options.read(true).write(true);
        let mut file = match directory.open_with(&part, options.clone().create_new(true)) {
            Ok(file) => {
                local_files::namespace_checkpoint(&directory)?.accept_receipt_reload_recovery();
                file.into_std()
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                directory.open_with(&part, &options)?.into_std()
            }
            Err(error) => return Err(error.into()),
        };
        validate_partial(&file, receipt.size)?;
        receipt.offset = receipt.offset.min(file.metadata()?.len());
        file.set_len(receipt.offset)?;
        file.sync_all()?;
        self.save(receipt, cancel).await?;
        while receipt.offset < receipt.size {
            ensure!(!cancel.is_cancelled(), "Transfer paused");
            let response = remote(
                receipt,
                "blob.read",
                json!({"blob_id":blob_id,"offset":receipt.offset}),
            )
            .await?;
            let metadata: Blob = serde_json::from_value(response["blob"].clone())?;
            metadata.validate(receipt)?;
            let encoded = response["data_hex"]
                .as_str()
                .context("Missing attachment bytes")?;
            ensure!(
                encoded.len() <= 512 * 1024,
                "Remote attachment chunk exceeds limit"
            );
            let bytes = hex::decode(encoded)?;
            let next = response["next_offset"]
                .as_u64()
                .context("Missing attachment offset")?;
            ensure!(
                !bytes.is_empty()
                    && next == receipt.offset + bytes.len() as u64
                    && next <= receipt.size,
                "Invalid attachment download offset"
            );
            ensure!(!cancel.is_cancelled(), "Transfer paused");
            directory.revalidate()?;
            validate_partial(&file, receipt.size)?;
            file.seek(SeekFrom::Start(receipt.offset))?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            receipt.offset = next;
            self.save(receipt, cancel).await?;
        }
        self.finish_download(receipt, file, directory, name, overwrite, cancel)
            .await
    }
    async fn finish_download(
        &self,
        receipt: &mut Receipt,
        mut file: std::fs::File,
        directory: local_files::ProtectedDirectory,
        name: String,
        overwrite: bool,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let size = receipt.size;
        let blob_id = receipt
            .blob_id
            .clone()
            .context("Download attachment identity missing")?;
        let part = local_files::part_name(&receipt.id);
        let (file, sha) = tokio::task::spawn_blocking(move || {
            let sha = local_files::hash(&mut file, size)?;
            Ok::<_, anyhow::Error>((file, sha))
        })
        .await??;
        if sha != receipt.sha256 {
            validate_partial(&file, receipt.size)?;
            file.set_len(0)?;
            file.sync_all()?;
            receipt.offset = 0;
            self.save(receipt, cancel).await?;
            anyhow::bail!(
                "Attachment SHA-256 mismatch; partial reset to zero, reselect destination to retry"
            );
        }
        let latest: Blob = serde_json::from_value(
            remote(receipt, "blob.status", json!({"blob_id":blob_id})).await?,
        )?;
        latest.validate(receipt)?;
        ensure!(
            latest.complete,
            "Attachment authorization changed before publication"
        );
        file.sync_all()?;
        validate_partial(&file, receipt.size)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let current = directory
                .open_with(&part, local_files::nofollow_options().read(true))?
                .into_std();
            let original = file.metadata()?;
            let current = current.metadata()?;
            ensure!(
                original.dev() == current.dev() && original.ino() == current.ino(),
                "Partial file identity changed; publication refused"
            );
        }
        let mut state = self.state.lock().await;
        ensure!(!cancel.is_cancelled(), "Transfer paused");
        receipt.state = "publishing".into();
        state.receipts.insert(receipt.id.clone(), receipt.clone());
        self.persist(&mut state)?;
        directory.publish_file(
            &file,
            part.to_str().context("Invalid partial filename")?,
            &name,
            overwrite,
        )?;
        receipt.state = "completed".into();
        state.receipts.insert(receipt.id.clone(), receipt.clone());
        self.persist(&mut state)
    }
}
fn validate_partial(file: &std::fs::File, size: u64) -> Result<()> {
    validate_cleanup_partial(file, size)?;
    #[cfg(windows)]
    local_files::windows::validate_partial(file)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            file.metadata()?.nlink() == 1,
            "Partial file link count changed"
        );
    }
    Ok(())
}

fn validate_cleanup_partial(file: &std::fs::File, size: u64) -> Result<()> {
    local_files::validate_file_acl(file)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= size,
        "Invalid local transfer partial file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0
                && metadata.nlink() >= 1,
            "Partial file ownership, permissions, or link count changed"
        );
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewRequest {
    pub connection_id: String,
    pub channel_id: String,
    pub blob_id: String,
}
fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}
impl TransferService {
    pub async fn preview(
        self: &Arc<Self>,
        request: PreviewRequest,
    ) -> Result<(&'static str, Vec<u8>)> {
        let _permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .context("Two file operations are active; retry preview shortly")?;
        let mut receipt = Receipt {
            id: id(),
            request_id: String::new(),
            direction: Direction::Download,
            binding: connection_binding(&request.connection_id).await?,
            connection_id: request.connection_id,
            channel_id: request.channel_id,
            blob_id: Some(request.blob_id.clone()),
            name: String::new(),
            size: 0,
            sha256: String::new(),
            offset: 0,
            state: "preview".into(),
            error: None,
            intent: String::new(),
            local_selection: String::new(),
            destination_identity: None,
        };
        let blob: Blob = serde_json::from_value(
            remote(&receipt, "blob.status", json!({"blob_id":request.blob_id})).await?,
        )?;
        blob.validate(&receipt)?;
        ensure!(
            blob.complete && blob.size <= 8 * 1024 * 1024,
            "Inline previews require a completed image of at most 8 MiB; save larger files instead"
        );
        ensure!(
            matches!(
                blob.media_type.as_str(),
                "image/png" | "image/jpeg" | "image/gif" | "image/webp"
            ),
            "Only PNG, JPEG, GIF and WebP attachments can be previewed inline"
        );
        receipt.size = blob.size;
        receipt.sha256 = blob.sha256;
        let mut bytes = Vec::with_capacity(receipt.size as usize);
        while receipt.offset < receipt.size {
            let response = remote(
                &receipt,
                "blob.read",
                json!({"blob_id":request.blob_id,"offset":receipt.offset}),
            )
            .await?;
            let metadata: Blob = serde_json::from_value(response["blob"].clone())?;
            metadata.validate(&receipt)?;
            let encoded = response["data_hex"]
                .as_str()
                .context("Missing image data")?;
            ensure!(encoded.len() <= 512 * 1024, "Image chunk exceeds limit");
            let chunk = hex::decode(encoded)?;
            let next = receipt.offset + chunk.len() as u64;
            ensure!(
                !chunk.is_empty()
                    && next <= receipt.size
                    && response["next_offset"].as_u64() == Some(next),
                "Invalid image chunk offset"
            );
            bytes.extend_from_slice(&chunk);
            receipt.offset = next;
        }
        ensure!(
            digest(&bytes) == receipt.sha256,
            "Image digest verification failed"
        );
        let media_type =
            image_type(&bytes).context("Attachment bytes are not an allowed image type")?;
        ensure!(
            media_type == blob.media_type,
            "Image type does not match attachment metadata"
        );
        let latest: Blob = serde_json::from_value(
            remote(&receipt, "blob.status", json!({"blob_id":request.blob_id})).await?,
        )?;
        latest.validate(&receipt)?;
        ensure!(
            latest.complete && latest.media_type == media_type,
            "Image metadata changed before preview"
        );
        Ok((media_type, bytes))
    }
}
