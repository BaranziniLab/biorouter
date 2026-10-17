//! Full-access remote operations (design OW-6, OW-7): what a run whose `run.remote_scope`
//! answers `access: "full"` may do through its owner's own bridge.
//!
//! Commands run as the owner's account through its login shell, with the network, child
//! processes and every path the account can reach, under a supervisor process
//! (`biorouter-crew __crew-run <job dir>`) that records each job in
//! `~/.local/state/biorouter-crew/jobs/<job id>/`. A job is keyed by its owner and idempotency
//! key, so any later run of the same person can read it, and a detached job outlives the bridge
//! and the grant. File operations take absolute, `~/` or work-folder-relative paths and follow
//! symlinks; nothing here checks ownership, because the account's own permissions are the
//! boundary. Two writes are refused anyway, as accident prevention rather than security: the
//! bridge executable (the running one and the installed `~/.local/bin/biorouter-crew`) and
//! `~/.ssh/authorized_keys`. Channel attachments move to and from the server in background
//! transfers over a second broker connection, so the relay never waits on them; at most
//! [`MAX_TRANSFERS`] run at once, inside the broker's connections per account.
//!
//! Every answer stays well under the 1 MiB frame: reads and writes move at most
//! [`MAX_CHUNK_BYTES`], output tails at most [`MAX_TAIL`] per stream.
use anyhow::{anyhow, bail, ensure, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::UnixStream,
    },
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// The most bytes one `remote.read` returns or one `remote.write` takes: hex-encoded, it fits a
/// frame with room to spare.
pub const MAX_CHUNK_BYTES: usize = 393_216;
/// A command's own default and largest run time, in seconds.
const DEFAULT_TIMEOUT: u64 = 600;
const MAX_TIMEOUT: u64 = 3600;
/// How long `remote.shell` waits for its command before answering `running`.
const DEFAULT_WAIT: u64 = 10;
const MAX_WAIT: u64 = 20;
/// Output kept in an answer, per stream.
const DEFAULT_TAIL: usize = 16_384;
const MAX_TAIL: usize = 65_536;
/// Jobs (commands and transfers) running at once in this account, and records kept.
const MAX_RUNNING: usize = 8;
/// Transfers running at once in this account. Each holds its own broker connection, and the
/// broker takes at most 8 per account, so this leaves room for the account's bridges.
const MAX_TRANSFERS: usize = 4;
const MAX_RECORDS: usize = 512;
/// The longest command text, or the most bytes of all argv items together.
const MAX_COMMAND: usize = 65_536;
const MAX_ARGV: usize = 256;
/// Directory entries one listing returns by default and at most.
const DEFAULT_LIST: usize = 500;
const MAX_LIST: usize = 2_000;
/// The largest attachment a transfer moves, as the broker allows.
const MAX_TRANSFER: u64 = 1 << 30;
/// The largest file `remote.hash` reads in one answer, which the relay waits for.
const MAX_HASH: u64 = 1 << 30;
/// How long a cancelled job's process group has between `SIGTERM` and `SIGKILL`.
const CANCEL_GRACE: Duration = Duration::from_secs(5);
/// A record with neither a process nor an exit this long after it was made never started.
const START_GRACE_SECS: u64 = 30;
/// The broker reads attachments in chunks of this size, and takes uploads in the same.
const TRANSFER_CHUNK: usize = 262_144;
/// A transfer writes its progress at most this often (in bytes moved).
const PROGRESS_EVERY: u64 = 4 * 1024 * 1024;

/// How a background transfer reaches the broker: a second connection, opened the way the
/// bridge opened its first, and the worker credential of the frame that asked for it.
pub type Connect = Arc<dyn Fn() -> Result<(UnixStream, BufReader<UnixStream>)> + Send + Sync>;

/// What a frame's operation may use to talk to the broker on its own.
#[derive(Clone)]
pub struct BrokerLink {
    pub connect: Connect,
    pub credential: String,
}

/// Where full-access operations run: the account's home (what `~/` means), the job records
/// directory, the running bridge's file (one of the files a structured write refuses), and
/// what the supervisor is started from.
#[derive(Clone, Debug)]
pub struct Context {
    pub home: PathBuf,
    pub jobs: PathBuf,
    /// The file the running bridge was started from, which a structured write refuses.
    pub executable: PathBuf,
    /// What `__crew-run` is started from: this process's own image.
    pub program: PathBuf,
}

impl Context {
    /// The bridge's own: `HOME` from its SSH login (the account database when unset), the file
    /// this process was started from (the installed `~/.local/bin/biorouter-crew` when it
    /// cannot tell), and its running image.
    pub fn of_this_process() -> Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|home| home.is_absolute())
            .map(Ok)
            .unwrap_or_else(|| crate::probe::this_account().map(|account| account.home))?;
        let executable = reported_executable().unwrap_or_else(|_| installed_executable(&home));
        Ok(Self {
            jobs: home.join(".local/state/biorouter-crew/jobs"),
            program: own_image(&executable),
            home,
            executable,
        })
    }
}

/// Where setup installs the bridge in an account.
fn installed_executable(home: &Path) -> PathBuf {
    home.join(".local/bin/biorouter-crew")
}

/// `path` without the ` (deleted)` Linux adds to `/proc/self/exe` once the file a process was
/// started from is gone, as when setup renames a newer copy over a running bridge.
fn without_deleted(path: PathBuf) -> PathBuf {
    match path
        .to_str()
        .and_then(|text| text.strip_suffix(" (deleted)"))
    {
        Some(text) => PathBuf::from(text),
        None => path,
    }
}

/// The path the running bridge was started from, as `current_exe` reports it, without
/// ` (deleted)`.
pub(crate) fn reported_executable() -> std::io::Result<PathBuf> {
    std::env::current_exe().map(without_deleted)
}

/// What a helper process of this bridge is started from: on Linux this process's own running
/// image (`/proc/self/exe`), which still runs after setup has replaced or removed the file it
/// was started from, and keeps a job of an older bridge from running a newer binary's
/// supervisor; elsewhere, or without `/proc`, `fallback`.
pub(crate) fn own_image(fallback: &Path) -> PathBuf {
    let image = Path::new("/proc/self/exe");
    if cfg!(target_os = "linux") && image.exists() {
        image.to_path_buf()
    } else {
        fallback.to_path_buf()
    }
}

fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .ok_or_else(|| anyhow!("invalid_params: {key} must be a string"))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Full access never runs as root: a root bridge would make every command the server's.
pub(crate) fn refuse_root(euid: u32) -> Result<()> {
    ensure!(
        euid != 0,
        "forbidden: Full access never runs commands as root."
    );
    Ok(())
}

/// A full-access operation, as the bridge's relay calls it with a scope the broker just gave.
pub fn handle(method: &str, params: &Value, scope: &Value, link: &BrokerLink) -> Result<Value> {
    refuse_root(unsafe { libc::geteuid() })?;
    handle_in(&Context::of_this_process()?, method, params, scope, link)
}

/// [`handle`] in a given [`Context`]; tests pass their own home and job directory.
pub fn handle_in(
    context: &Context,
    method: &str,
    params: &Value,
    scope: &Value,
    link: &BrokerLink,
) -> Result<Value> {
    ensure!(
        scope["public_provider"] == false,
        "forbidden: public providers cannot access remote files or jobs"
    );
    ensure!(
        scope["access"] == "full",
        "invalid_scope: this run does not have full access"
    );
    let owner = text(scope, "owner_id")
        .ok()
        .filter(|owner| !owner.is_empty())
        .ok_or_else(|| anyhow!("invalid_scope: full access needs the run's owner"))?;
    let root = PathBuf::from(text(scope, "root")?);
    ensure!(root.is_absolute(), "invalid_scope: root must be absolute");
    match method {
        "remote.shell" | "remote.execute" => start_command(context, &root, method, params, scope),
        "remote.job_status" => {
            let dir = job_dir(context, params, owner)?;
            job_view(&dir, tail_bytes(params)?)
        }
        "remote.cancel" => cancel(&job_dir(context, params, owner)?),
        "remote.list" => list(&resolve(context, &root, params)?, params),
        "remote.stat" => stat(&resolve(context, &root, params)?),
        "remote.read" => read(&resolve(context, &root, params)?, params),
        "remote.write" => write(context, &resolve(context, &root, params)?, params),
        "remote.hash" => hash(&resolve(context, &root, params)?),
        "remote.fetch_blob" => fetch_blob(context, &root, params, scope, link),
        "remote.share_file" => share_file(context, &root, params, scope, link),
        _ => bail!("unsupported: remote operation unavailable"),
    }
}

/// Where `path` points: `~` and `~/…` inside the home, an absolute path as it is, anything
/// else inside the work folder `root`. Nothing is resolved further: symlinks are followed by
/// the operation that opens the path.
pub fn resolve_path(home: &Path, root: &Path, path: &str) -> Result<PathBuf> {
    ensure!(
        !path.contains('\0') && path.len() <= 4096,
        "invalid_params: path must be at most 4096 bytes without NUL"
    );
    Ok(if path == "~" {
        home.to_path_buf()
    } else if let Some(inside) = path.strip_prefix("~/") {
        home.join(inside)
    } else if path.starts_with('~') {
        bail!("invalid_params: ~user paths are not supported; give an absolute path instead.")
    } else if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        root.join(path)
    })
}

fn resolve(context: &Context, root: &Path, params: &Value) -> Result<PathBuf> {
    let path = match params.get("path") {
        None | Some(Value::Null) => ".",
        Some(Value::String(path)) => path.as_str(),
        Some(_) => bail!("invalid_params: path must be a string"),
    };
    resolve_path(&context.home, root, path)
}

/// `path` with `.` and `..` folded away lexically, for comparing a path that does not exist yet.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// `path` as given and, when it resolves, as the filesystem resolves it.
fn spellings(path: &Path) -> Vec<PathBuf> {
    let mut variants = vec![lexical(path)];
    if let Ok(resolved) = fs::canonicalize(path) {
        if !variants.contains(&resolved) {
            variants.push(resolved);
        }
    }
    variants
}

/// The files a structured write refuses, under every spelling: the running bridge and the
/// installed one, and the account's `authorized_keys` files. The shell can still change them;
/// this keeps an agent's ordinary file write from locking the person out by accident.
fn protected_writes(context: &Context) -> Vec<PathBuf> {
    let mut protected = spellings(&context.executable);
    for installed in spellings(&installed_executable(&context.home)) {
        if !protected.contains(&installed) {
            protected.push(installed);
        }
    }
    for home in spellings(&context.home) {
        for name in ["authorized_keys", "authorized_keys2"] {
            protected.push(home.join(".ssh").join(name));
        }
    }
    protected
}

/// Refuse a write whose target, however it is spelled or linked, is a protected file.
pub(crate) fn refuse_protected_write(context: &Context, target: &Path) -> Result<()> {
    let resolved = match fs::canonicalize(target) {
        Ok(resolved) => resolved,
        Err(_) => {
            ensure!(
                fs::symlink_metadata(target).is_err(),
                "invalid_params: {} is a symbolic link to nothing; write to a real path instead.",
                target.display()
            );
            let lexical_target = lexical(target);
            match (lexical_target.parent(), lexical_target.file_name()) {
                (Some(parent), Some(name)) => fs::canonicalize(parent)
                    .map(|parent| parent.join(name))
                    .unwrap_or(lexical_target.clone()),
                _ => lexical_target,
            }
        }
    };
    let lexical_target = lexical(target);
    for protected in protected_writes(context) {
        ensure!(
            resolved != protected && lexical_target != protected,
            "forbidden: Crew does not write {}, which keeps your Crew bridge or SSH sign-in working. Change it yourself if you mean to.",
            protected.display()
        );
    }
    Ok(())
}

fn mtime(metadata: &fs::Metadata) -> i64 {
    metadata.mtime()
}

fn kind_of(file_type: fs::FileType) -> &'static str {
    if file_type.is_symlink() {
        "symlink"
    } else if file_type.is_dir() {
        "directory"
    } else if file_type.is_file() {
        "file"
    } else {
        "other"
    }
}

fn list(path: &Path, params: &Value) -> Result<Value> {
    let limit = match params.get("limit") {
        None | Some(Value::Null) => DEFAULT_LIST,
        Some(value) => value
            .as_u64()
            .filter(|limit| (1..=MAX_LIST as u64).contains(limit))
            .ok_or_else(|| anyhow!("invalid_params: limit must be 1..{MAX_LIST}"))?
            as usize,
    };
    let mut entries = Vec::new();
    let mut truncated = false;
    for entry in fs::read_dir(path).map_err(|e| anyhow!("remote path unavailable: {e}"))? {
        if entries.len() == limit {
            truncated = true;
            break;
        }
        let entry = entry?;
        let metadata = entry.metadata()?;
        let mut item = json!({
            "name": entry.file_name().to_string_lossy(),
            "type": kind_of(metadata.file_type()),
            "size": metadata.len(),
            "mtime": mtime(&metadata),
            "mode": format!("{:04o}", metadata.mode() & 0o7777),
        });
        if metadata.file_type().is_symlink() {
            if let Ok(target) = fs::metadata(entry.path()) {
                item["target_type"] = json!(kind_of(target.file_type()));
            }
        }
        entries.push(item);
    }
    entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(json!({"path": path, "entries": entries, "truncated": truncated}))
}

fn stat(path: &Path) -> Result<Value> {
    let Ok(link) = fs::symlink_metadata(path) else {
        return Ok(json!({"path": path, "exists": false}));
    };
    let symlink = link.file_type().is_symlink();
    let Ok(metadata) = fs::metadata(path) else {
        return Ok(
            json!({"path": path, "exists": false, "symlink": true, "target": fs::read_link(path).ok()}),
        );
    };
    let mut answer = json!({
        "path": path,
        "exists": true,
        "type": kind_of(metadata.file_type()),
        "size": metadata.len(),
        "mtime": mtime(&metadata),
        "mode": format!("{:04o}", metadata.mode() & 0o7777),
        "uid": metadata.uid(),
        "gid": metadata.gid(),
        "symlink": symlink,
    });
    if symlink {
        answer["target"] = json!(fs::read_link(path).ok());
    }
    Ok(answer)
}

/// `remote.read {path, offset?, length?}`: at most [`MAX_CHUNK_BYTES`] from `offset`. The bytes
/// come back as hex, and also as text when they are UTF-8 and the frame has room for both.
///
/// A read with neither `offset` nor `length` is a read of the whole file, as in folder mode: a
/// file of at most [`crate::MAX_CHUNK`] (256 KiB, what `remote.attach` moves to the channel in
/// one piece), and a larger one is refused before anything is read, rather than answered with
/// its first part, which `size` and `sha256` would then describe as if it were the file.
fn read(path: &Path, params: &Value) -> Result<Value> {
    let absent = |key: &str| params.get(key).is_none_or(Value::is_null);
    let whole = absent("offset") && absent("length");
    let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0);
    let length = match params.get("length") {
        None | Some(Value::Null) => MAX_CHUNK_BYTES,
        Some(value) => value
            .as_u64()
            .filter(|length| *length <= MAX_CHUNK_BYTES as u64)
            .ok_or_else(|| anyhow!("invalid_params: length must be 0..{MAX_CHUNK_BYTES}"))?
            as usize,
    };
    let mut file = File::open(path).map_err(|e| anyhow!("remote path unavailable: {e}"))?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "not a regular file");
    let file_size = metadata.len();
    ensure!(
        !whole || file_size <= crate::MAX_CHUNK as u64,
        "invalid_params: {} is {file_size} bytes, larger than a read without offset and length returns (256 KiB). Read it in parts with offset and length (at most {MAX_CHUNK_BYTES} bytes each), or post it to the channel with remote.share_file.",
        path.display()
    );
    ensure!(
        offset <= file_size,
        "invalid_params: offset beyond the end of the file"
    );
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::with_capacity(length.min(file_size.saturating_sub(offset) as usize));
    (&mut file).take(length as u64).read_to_end(&mut bytes)?;
    let next_offset = offset + bytes.len() as u64;
    let data_hex = hex::encode(&bytes);
    let mut answer = json!({
        "path": path,
        "offset": offset,
        "size": bytes.len(),
        "file_size": file_size,
        "next_offset": next_offset,
        "eof": next_offset >= file_size,
        "sha256": hex::encode(Sha256::digest(&bytes)),
        "data_hex": data_hex,
    });
    if let Ok(text) = std::str::from_utf8(&bytes) {
        // Escaped text can be six times its bytes; it rides along only while the frame has room.
        if serde_json::to_string(text)?.len() + data_hex.len() < 960_000 {
            answer["text_utf8"] = json!(text);
        }
    }
    Ok(answer)
}

/// `remote.write {path, text | data_hex, mode?, create_parents?}`: `truncate` (the default)
/// replaces the file, `append` adds to it and `create_new` refuses one that exists.
fn write(context: &Context, path: &Path, params: &Value) -> Result<Value> {
    ensure!(
        params.get("text").is_some() != params.get("data_hex").is_some(),
        "invalid_params: provide exactly one of text or data_hex"
    );
    let bytes = if let Some(text) = params.get("text") {
        text.as_str()
            .ok_or_else(|| anyhow!("invalid_params: text must be a string"))?
            .as_bytes()
            .to_vec()
    } else {
        hex::decode(text(params, "data_hex")?)
            .map_err(|_| anyhow!("invalid_params: data_hex must be hexadecimal"))?
    };
    ensure!(
        bytes.len() <= MAX_CHUNK_BYTES,
        "invalid_params: a write takes at most {MAX_CHUNK_BYTES} bytes; append the rest"
    );
    let mode = params
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("truncate");
    let mut options = OpenOptions::new();
    options.write(true);
    match mode {
        "truncate" => options.create(true).truncate(true),
        "append" => options.create(true).append(true),
        "create_new" => options.create_new(true),
        _ => bail!("invalid_params: mode must be truncate, append or create_new"),
    };
    refuse_protected_write(context, path)?;
    if params.get("create_parents").and_then(Value::as_bool) == Some(true) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| anyhow!("remote path unavailable: {e}"))?;
        }
    }
    let mut file = options.open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            anyhow!("conflict: {} already exists", path.display())
        } else {
            anyhow!("remote path unavailable: {e}")
        }
    })?;
    ensure!(file.metadata()?.is_file(), "not a regular file");
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(json!({
        "path": path,
        "mode": mode,
        "size": bytes.len(),
        "file_size": file.metadata()?.len(),
        "sha256": hex::encode(Sha256::digest(&bytes)),
    }))
}

fn hash(path: &Path) -> Result<Value> {
    let mut file = File::open(path).map_err(|e| anyhow!("remote path unavailable: {e}"))?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "not a regular file");
    ensure!(
        metadata.len() <= MAX_HASH,
        "invalid_params: remote.hash reads files up to 1 GiB; run sha256sum with remote.shell for larger ones"
    );
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        hasher.update(&buffer[..n]);
    }
    Ok(json!({"path": path, "sha256": hex::encode(hasher.finalize()), "size": size}))
}

/// The job ID of `key` for `owner`: SHA-256 of `owner:key`, so any run of the same person reaches
/// the same job and nobody else's.
pub fn job_id(owner: &str, key: &str) -> String {
    hex::encode(Sha256::digest(format!("{owner}:{key}").as_bytes()))
}

fn idempotency_key(params: &Value) -> Result<&str> {
    let key = text(params, "idempotency_key")?;
    ensure!(
        !key.is_empty() && key.len() <= 128,
        "invalid_params: idempotency_key must be 1 to 128 bytes"
    );
    Ok(key)
}

fn tail_bytes(params: &Value) -> Result<usize> {
    match params.get("tail_bytes") {
        None | Some(Value::Null) => Ok(DEFAULT_TAIL),
        Some(value) => value
            .as_u64()
            .filter(|tail| *tail <= MAX_TAIL as u64)
            .map(|tail| tail as usize)
            .ok_or_else(|| anyhow!("invalid_params: tail_bytes must be 0..{MAX_TAIL}")),
    }
}

/// The job directory a status or cancel names, refused unless the job is `owner`'s.
fn job_dir(context: &Context, params: &Value, owner: &str) -> Result<PathBuf> {
    let id = text(params, "job_id")?;
    ensure!(
        id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid_params: invalid job ID"
    );
    let dir = context.jobs.join(id.to_ascii_lowercase());
    let record = read_json(&dir.join("job.json"))
        .map_err(|_| anyhow!("not_found: No Crew job has this ID in this account."))?;
    ensure!(
        record["owner_id"] == owner,
        "forbidden: job belongs to another person"
    );
    Ok(dir)
}

/// A small JSON record the bridge or a supervisor wrote, never through a symlink.
fn read_json(path: &Path) -> Result<Value> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let mut data = Vec::new();
    file.take(1_048_577).read_to_end(&mut data)?;
    ensure!(data.len() <= 1_048_576, "invalid job record");
    Ok(serde_json::from_slice(&data)?)
}

/// Replace `path` with `value` atomically: a private temporary file beside it, synced, renamed.
fn write_json(path: &Path, value: &Value) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("job state directory missing"))?;
    let temporary = parent.join(format!(".record-{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&temporary)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

/// The jobs directory, created owner-only, refused when it is a symlink or someone else's.
fn jobs_directory(context: &Context) -> Result<&Path> {
    let jobs = context.jobs.as_path();
    if let Some(parent) = jobs.parent() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    match fs::DirBuilder::new().mode(0o700).create(jobs) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(jobs)?;
    ensure!(
        metadata.is_dir() && metadata.uid() == unsafe { libc::geteuid() },
        "unsafe_storage: the Crew jobs directory must be this account's own directory"
    );
    if metadata.mode() & 0o077 != 0 {
        fs::set_permissions(jobs, fs::Permissions::from_mode(0o700))?;
    }
    Ok(jobs)
}

/// `/proc/<pid>/stat`'s state letter and start time (field 22, in clock ticks since boot).
fn process_identity(pid: u32) -> Option<(char, u64)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name is in parentheses and may hold spaces or parentheses itself.
    let rest = stat.get(stat.rfind(')')? + 1..)?;
    let mut fields = rest.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let start = fields.nth(18)?.parse().ok()?;
    Some((state, start))
}

/// Whether `pid` is still the process that started at `start` and has not exited.
fn alive(pid: u64, start: u64) -> bool {
    u32::try_from(pid)
        .ok()
        .and_then(process_identity)
        .is_some_and(|(state, started)| started == start && state != 'Z' && state != 'X')
}

fn alive_in(record: &Value, pid: &str, start: &str) -> bool {
    match (record[pid].as_u64(), record[start].as_u64()) {
        (Some(pid), Some(start)) => alive(pid, start),
        _ => false,
    }
}

/// The last `tail` bytes of an output file, and whether there was more.
fn tail_of(path: &Path, tail: usize) -> (String, bool) {
    let Ok(mut file) = File::open(path) else {
        return (String::new(), false);
    };
    let length = file.metadata().map(|m| m.len()).unwrap_or(0);
    let start = length.saturating_sub(tail as u64);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return (String::new(), false);
    }
    let mut bytes = Vec::new();
    let _ = file.take(tail as u64).read_to_end(&mut bytes);
    (String::from_utf8_lossy(&bytes).into_owned(), start > 0)
}

/// A job's state as its record, process and exit files tell it: `starting`, `running`,
/// `completed`, `failed`, `timed_out`, `cancelled`, or `lost` when the process that ran it is
/// gone without saying how it ended.
fn job_state(dir: &Path, record: &Value) -> (String, Option<Value>) {
    if let Ok(exit) = read_json(&dir.join("exit.json")) {
        let state = exit["state"].as_str().unwrap_or("failed").to_owned();
        return (state, Some(exit));
    }
    if let Ok(process) = read_json(&dir.join("process.json")) {
        let running = alive_in(&process, "pid", "start_time")
            || alive_in(&process, "supervisor_pid", "supervisor_start_time");
        return (if running { "running" } else { "lost" }.into(), None);
    }
    let created = record["created_at"].as_u64().unwrap_or(0);
    let state = if now().saturating_sub(created) < START_GRACE_SECS {
        "starting"
    } else {
        "lost"
    };
    (state.into(), None)
}

fn is_finished(state: &str) -> bool {
    !matches!(state, "starting" | "running")
}

/// What `remote.job_status` (and every start) answers for the job in `dir`.
fn job_view(dir: &Path, tail: usize) -> Result<Value> {
    let record = read_json(&dir.join("job.json"))?;
    let (state, exit) = job_state(dir, &record);
    let mut answer = json!({
        "job_id": record["job_id"],
        "kind": record["kind"],
        "state": state,
        "detach": record["detach"],
        "created_at": record["created_at"],
    });
    let exit = exit.unwrap_or(Value::Null);
    for key in ["exit_code", "signal", "ended_at", "error", "message"] {
        if !exit[key].is_null() {
            answer[key] = exit[key].clone();
        }
    }
    match record["kind"].as_str() {
        Some("shell" | "execute") => {
            let (stdout_tail, stdout_cut) = tail_of(&dir.join("stdout"), tail);
            let (stderr_tail, stderr_cut) = tail_of(&dir.join("stderr"), tail);
            answer["stdout_tail"] = json!(stdout_tail);
            answer["stderr_tail"] = json!(stderr_tail);
            answer["truncated"] = json!(stdout_cut || stderr_cut);
            answer["output_dir"] = json!(dir);
        }
        _ => {
            let progress = read_json(&dir.join("progress.json")).unwrap_or(Value::Null);
            answer["bytes_done"] = progress["bytes_done"].clone();
            answer["size"] = progress["size"].clone();
            for key in ["path", "blob_id", "name"] {
                if !record[key].is_null() {
                    answer[key] = record[key].clone();
                }
            }
            if !exit["result"].is_null() {
                answer["result"] = exit["result"].clone();
                if let Some(result) = exit["result"].as_object() {
                    for (key, value) in result {
                        answer[key] = value.clone();
                    }
                }
            }
        }
    }
    if state == "lost" && answer.get("message").is_none() {
        answer["message"] = json!(
            "The process that ran this job is gone and left no exit status. Inspect its output before retrying."
        );
    }
    Ok(answer)
}

/// Whether a job record's `kind` is a transfer, which holds a broker connection while it runs.
fn is_transfer(kind: Option<&str>) -> bool {
    matches!(kind, Some("fetch_blob" | "share_file"))
}

/// Make room for one more job (a transfer when `transfer`): refuse while [`MAX_RUNNING`] run,
/// or a transfer while [`MAX_TRANSFERS`] do, and drop the oldest finished records once
/// [`MAX_RECORDS`] are kept.
fn make_room(jobs: &Path, transfer: bool) -> Result<()> {
    let mut finished = Vec::new();
    let mut records = 0usize;
    let mut running = 0usize;
    let mut transfers = 0usize;
    for entry in fs::read_dir(jobs)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.len() != 64 || !name.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        records += 1;
        let dir = entry.path();
        let Ok(record) = read_json(&dir.join("job.json")) else {
            continue;
        };
        let (state, exit) = job_state(&dir, &record);
        if is_finished(&state) {
            let ended = exit
                .and_then(|exit| exit["ended_at"].as_u64())
                .or_else(|| record["created_at"].as_u64())
                .unwrap_or(0);
            finished.push((ended, dir));
        } else {
            running += 1;
            transfers += usize::from(is_transfer(record["kind"].as_str()));
        }
    }
    ensure!(
        running < MAX_RUNNING,
        "quota_exceeded: {MAX_RUNNING} Crew jobs are already running in this account. Wait for one to finish or cancel one."
    );
    ensure!(
        !transfer || transfers < MAX_TRANSFERS,
        "quota_exceeded: {MAX_TRANSFERS} Crew file copies are already running in this account. Wait for one to finish (remote.job_status) or cancel one."
    );
    if records >= MAX_RECORDS {
        finished.sort();
        let excess = records + 1 - MAX_RECORDS;
        ensure!(
            finished.len() >= excess,
            "quota_exceeded: This account keeps {MAX_RECORDS} Crew job records and none has finished."
        );
        for (_, dir) in finished.into_iter().take(excess) {
            let _ = fs::remove_dir_all(dir);
        }
    }
    Ok(())
}

/// Create the job's record directory with `record` as its `job.json`, or find the one an earlier
/// request with the same key made: `Ok(None)` when created, `Ok(Some(dir))` when it existed and
/// asked for the same thing.
fn reserve(context: &Context, id: &str, record: &Value) -> Result<(PathBuf, bool)> {
    let jobs = jobs_directory(context)?;
    let dir = jobs.join(id);
    if dir.exists() {
        let existing = read_json(&dir.join("job.json"))?;
        ensure!(
            existing["owner_id"] == record["owner_id"],
            "forbidden: job belongs to another person"
        );
        ensure!(
            existing["digest"] == record["digest"],
            "conflict: idempotency key reused with a different request"
        );
        return Ok((dir, false));
    }
    make_room(jobs, is_transfer(record["kind"].as_str()))?;
    let temporary = jobs.join(format!(".new-{}", uuid::Uuid::new_v4().simple()));
    fs::DirBuilder::new().mode(0o700).create(&temporary)?;
    let made = (|| -> Result<()> {
        write_json(&temporary.join("job.json"), record)?;
        fs::rename(&temporary, &dir)?;
        File::open(jobs)?.sync_all()?;
        Ok(())
    })();
    if let Err(error) = made {
        let _ = fs::remove_dir_all(&temporary);
        // Another bridge of the same person made it first: answer with theirs.
        if dir.join("job.json").exists() {
            return reserve(context, id, record);
        }
        return Err(error);
    }
    Ok((dir, true))
}

/// The account's login shell from the account database, or `/bin/sh`.
fn login_shell() -> PathBuf {
    crate::probe::this_account()
        .ok()
        .and_then(|account| account.shell)
        .filter(|shell| shell.exists())
        .unwrap_or_else(|| PathBuf::from("/bin/sh"))
}

fn shell_name(shell: &Path) -> &str {
    shell.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

/// The argv a job runs: `<shell> -lc <command>`, or for an argv the login shell running it
/// exactly (`exec "$@"`), so modules, conda and `PATH` behave as in the person's own login.
/// csh-family shells take `-l` only alone, so they get `-c`; an argv under a shell that is not
/// of the Bourne family goes through `/bin/sh -l`.
pub(crate) fn job_argv(
    shell: &Path,
    command: Option<&str>,
    argv: Option<&[String]>,
) -> Vec<String> {
    let name = shell_name(shell);
    let shell_text = shell.to_string_lossy().into_owned();
    match (command, argv) {
        (Some(command), _) => {
            let flag = if matches!(name, "csh" | "tcsh") {
                "-c"
            } else {
                "-lc"
            };
            vec![shell_text, flag.into(), command.into()]
        }
        (None, Some(argv)) => {
            let posix = matches!(name, "sh" | "bash" | "zsh" | "dash" | "ksh" | "mksh");
            let runner = if posix { shell_text } else { "/bin/sh".into() };
            let mut out = vec![
                runner,
                "-lc".into(),
                "exec \"$@\"".into(),
                "biorouter-crew-job".into(),
            ];
            out.extend(argv.iter().cloned());
            out
        }
        (None, None) => Vec::new(),
    }
}

fn start_command(
    context: &Context,
    root: &Path,
    method: &str,
    params: &Value,
    scope: &Value,
) -> Result<Value> {
    ensure!(
        scope["allow_execute"] == true,
        "forbidden: remote execution requires an explicit human grant"
    );
    let expiry = scope["expires_at"]
        .as_u64()
        .ok_or_else(|| anyhow!("grant expiry missing"))?;
    ensure!(expiry > now(), "remote execution grant expired");
    let key = idempotency_key(params)?;
    let detach = match params.get("detach") {
        None | Some(Value::Null) => false,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| anyhow!("invalid_params: detach must be true or false"))?,
    };
    let timeout = match params.get("timeout_seconds") {
        None | Some(Value::Null) => DEFAULT_TIMEOUT,
        Some(value) => value
            .as_u64()
            .filter(|seconds| (1..=MAX_TIMEOUT).contains(seconds))
            .ok_or_else(|| anyhow!("invalid_params: timeout_seconds must be 1..{MAX_TIMEOUT}"))?,
    };
    let wait = params
        .get("wait_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_WAIT)
        .min(MAX_WAIT);
    let cwd = match params.get("cwd") {
        None | Some(Value::Null) => root.to_path_buf(),
        Some(Value::String(cwd)) => resolve_path(&context.home, root, cwd)?,
        Some(_) => bail!("invalid_params: cwd must be a string"),
    };
    ensure!(
        cwd.is_dir(),
        "invalid_params: the working directory {} does not exist",
        cwd.display()
    );
    let (kind, command, argv) = if method == "remote.shell" {
        let command = text(params, "command")?;
        ensure!(
            !command.trim().is_empty() && command.len() <= MAX_COMMAND && !command.contains('\0'),
            "invalid_params: command must be 1 to {MAX_COMMAND} bytes without NUL"
        );
        ("shell", Some(command.to_owned()), None)
    } else {
        let argv: Vec<String> = serde_json::from_value(params["argv"].clone())
            .map_err(|_| anyhow!("invalid_params: argv must be a list of strings"))?;
        ensure!(
            !argv.is_empty()
                && argv.len() <= MAX_ARGV
                && argv.iter().map(String::len).sum::<usize>() <= MAX_COMMAND
                && argv.iter().all(|item| !item.contains('\0')),
            "invalid_params: bounded argv required"
        );
        ("execute", None, Some(argv))
    };
    let shell = login_shell();
    let spec = json!({"kind": kind, "command": command, "argv": argv, "cwd": cwd, "timeout_seconds": timeout, "detach": detach});
    let owner = text(scope, "owner_id")?;
    let id = job_id(owner, key);
    let record = json!({
        "job_id": id,
        "kind": kind,
        "owner_id": owner,
        "run_id": scope["run_id"],
        "digest": hex::encode(Sha256::digest(serde_json::to_vec(&spec)?)),
        "created_at": now(),
        "detach": detach,
        "timeout_seconds": timeout,
        "cwd": cwd,
        "argv": job_argv(&shell, command.as_deref(), argv.as_deref()),
        "parent_pid": std::process::id(),
    });
    let (dir, created) = reserve(context, &id, &record)?;
    if created {
        if let Err(error) = spawn_supervisor(context, &dir, &id) {
            let _ = write_json(
                &dir.join("exit.json"),
                &json!({"state": "failed", "ended_at": now(), "error": format!("The job could not start: {error}")}),
            );
        }
    }
    let deadline = Instant::now() + Duration::from_secs(wait);
    let record = read_json(&dir.join("job.json"))?;
    while !is_finished(&job_state(&dir, &record).0) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    job_view(&dir, DEFAULT_TAIL)
}

/// Start `biorouter-crew __crew-run <dir>`. Its standard streams are `/dev/null`, never the
/// bridge's own (standard output is the frame channel to the daemon). It is reaped on a thread:
/// a job that is not detached stays this bridge's child, and dies with it.
fn spawn_supervisor(context: &Context, dir: &Path, id: &str) -> Result<()> {
    let mut child = std::process::Command::new(&context.program)
        .arg("__crew-run")
        .arg(dir)
        .env("BIOROUTER_CREW_JOB_ID", id)
        .current_dir("/")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Send `signal` to the job's process group, only while its leader is still the process the
/// supervisor started.
fn signal_group(process: &Value, signal: libc::c_int) -> bool {
    if !alive_in(process, "pid", "start_time") {
        return false;
    }
    let Some(pgid) = process["pgid"].as_i64().and_then(|p| i32::try_from(p).ok()) else {
        return false;
    };
    pgid > 1 && unsafe { libc::kill(-pgid, signal) } == 0
}

/// `remote.cancel`: a command's process group gets `SIGTERM`, then `SIGKILL` after
/// [`CANCEL_GRACE`]; a transfer stops at its next chunk. A finished job is left as it is.
fn cancel(dir: &Path) -> Result<Value> {
    let record = read_json(&dir.join("job.json"))?;
    let (state, _) = job_state(dir, &record);
    if is_finished(&state) {
        return job_view(dir, DEFAULT_TAIL);
    }
    write_json(&dir.join("cancel.json"), &json!({"requested_at": now()}))?;
    if matches!(record["kind"].as_str(), Some("shell" | "execute")) {
        if let Ok(process) = read_json(&dir.join("process.json")) {
            if signal_group(&process, libc::SIGTERM) {
                let deadline = Instant::now() + CANCEL_GRACE;
                while alive_in(&process, "pid", "start_time") && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(100));
                }
                signal_group(&process, libc::SIGKILL);
            }
        }
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while !dir.join("exit.json").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    job_view(dir, DEFAULT_TAIL)
}

/// The supervisor (`biorouter-crew __crew-run <job dir>`): runs one job's argv as this account
/// in its own process group, appends its output to the job's files, enforces its timeout and
/// writes `exit.json`. A job that is not detached ends with the bridge that started it.
pub fn supervise(dir: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux_supervise(dir)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = dir;
        bail!("unsupported: Crew jobs require Linux")
    }
}

#[cfg(target_os = "linux")]
static STOP_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "linux")]
extern "C" fn request_stop(_: libc::c_int) {
    STOP_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(target_os = "linux")]
fn linux_supervise(dir: &Path) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let metadata = fs::symlink_metadata(dir)?;
    ensure!(
        metadata.is_dir()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "unsafe_storage: job directory must be this account's own private directory"
    );
    let record = read_json(&dir.join("job.json"))?;
    let finish = |state: &str, extra: Value| {
        let mut exit = json!({"state": state, "ended_at": now()});
        if let (Some(exit), Some(extra)) = (exit.as_object_mut(), extra.as_object()) {
            exit.extend(extra.clone());
        }
        write_json(&dir.join("exit.json"), &exit)
    };
    let detach = record["detach"] == true;
    if detach {
        // The second fork: this process exits at once, so the bridge reaps it, and the
        // supervisor that carries on is nobody's child but init's (or a subreaper's).
        match unsafe { libc::fork() } {
            -1 => return Err(std::io::Error::last_os_error().into()),
            0 => {}
            _ => return Ok(()),
        }
    } else {
        unsafe {
            ensure!(
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == 0,
                "parent lifecycle unavailable"
            );
        }
        if record["parent_pid"].as_u64() != Some(unsafe { libc::getppid() } as u64) {
            return finish(
                "cancelled",
                json!({"message": "The Crew connection ended before the job started."}),
            );
        }
    }
    unsafe {
        libc::setsid();
        for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::signal(
                signal,
                request_stop as extern "C" fn(libc::c_int) as libc::sighandler_t,
            );
        }
    }
    let argv: Vec<String> = serde_json::from_value(record["argv"].clone())?;
    ensure!(!argv.is_empty(), "argv missing");
    let output = |name: &str| {
        OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(dir.join(name))
    };
    let (stdout, stderr) = match (output("stdout"), output("stderr")) {
        (Ok(stdout), Ok(stderr)) => (stdout, stderr),
        (Err(error), _) | (_, Err(error)) => {
            return finish(
                "failed",
                json!({"error": format!("The job's output files could not be opened: {error}")}),
            );
        }
    };
    let cwd = record["cwd"].as_str().unwrap_or("/").to_owned();
    let mut command = std::process::Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(&cwd)
        .stdin(std::process::Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    unsafe {
        command.pre_exec(move || {
            // Its own process group, so a cancel or timeout reaches everything it starts and
            // spares this supervisor; and a command the person did not detach dies with it.
            if libc::setpgid(0, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if !detach {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            }
            Ok(())
        });
    }
    // A cancel that arrived before the command started stops it from starting at all.
    if dir.join("cancel.json").exists() {
        return finish("cancelled", json!({}));
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return finish(
                "failed",
                json!({"error": format!("The command could not start: {error}")}),
            );
        }
    };
    let pid = child.id();
    let identity = |pid: u32| process_identity(pid).map(|(_, start)| start);
    write_json(
        &dir.join("process.json"),
        &json!({
            "pid": pid,
            "pgid": pid,
            "start_time": identity(pid),
            "supervisor_pid": std::process::id(),
            "supervisor_start_time": identity(std::process::id()),
            "started_at": now(),
        }),
    )?;
    let timeout = Duration::from_secs(
        record["timeout_seconds"]
            .as_u64()
            .unwrap_or(DEFAULT_TIMEOUT),
    );
    let deadline = Instant::now() + timeout;
    let mut stopped: Option<&str> = None;
    let mut kill_at: Option<Instant> = None;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        // A cancel the bridge recorded before this process's group was known to it (or that
        // could not signal it) is honored here.
        let stop = STOP_REQUESTED.load(std::sync::atomic::Ordering::SeqCst)
            || dir.join("cancel.json").exists();
        if stopped.is_none() && (stop || Instant::now() >= deadline) {
            stopped = Some(if stop { "cancelled" } else { "timed_out" });
            unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
            kill_at = Some(Instant::now() + CANCEL_GRACE);
        }
        if kill_at.is_some_and(|at| Instant::now() >= at) {
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
            kill_at = None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    use std::os::unix::process::ExitStatusExt;
    let cancelled = dir.join("cancel.json").exists();
    let state = if cancelled {
        "cancelled"
    } else if let Some(stopped) = stopped {
        stopped
    } else if status.success() {
        "completed"
    } else {
        "failed"
    };
    let mut extra = json!({"exit_code": status.code(), "signal": status.signal()});
    if state == "cancelled" && !cancelled {
        extra["message"] = json!(
            "Stopped because the Crew connection ended. Use detach for work that must outlive the connection."
        );
    } else if state == "timed_out" {
        extra["message"] = json!(format!(
            "Stopped after its timeout of {} seconds.",
            timeout.as_secs()
        ));
    }
    finish(state, extra)
}

/// Write a transfer's record files for this bridge process, which runs it on a thread.
fn transfer_process(dir: &Path) -> Result<()> {
    let pid = std::process::id();
    write_json(
        &dir.join("process.json"),
        &json!({
            "pid": pid,
            "start_time": process_identity(pid).map(|(_, start)| start),
            "started_at": now(),
        }),
    )
}

/// One request on a transfer's own broker connection, answered or refused with the broker's
/// own code and sentence.
fn call(
    stream: &mut UnixStream,
    reader: &mut BufReader<UnixStream>,
    credential: &str,
    sequence: &mut u64,
    method: &str,
    params: Value,
) -> Result<Value> {
    *sequence += 1;
    let request = json!({
        "version": crate::PROTOCOL_VERSION,
        "id": format!("transfer-{sequence}"),
        "method": method,
        "params": params,
        "credential": credential,
    });
    let mut bytes = serde_json::to_vec(&request)?;
    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    stream.flush()?;
    let mut frame = Vec::new();
    let n = reader
        .take((crate::MAX_FRAME + 1) as u64)
        .read_until(b'\n', &mut frame)?;
    ensure!(
        n > 0 && n <= crate::MAX_FRAME && frame.ends_with(b"\n"),
        "broker_unavailable: the workspace server closed the transfer connection"
    );
    let response: crate::Response = serde_json::from_slice(&frame)?;
    if let Some(error) = response.error {
        bail!("{}", error.message);
    }
    response
        .result
        .ok_or_else(|| anyhow!("invalid_response: the workspace server sent no result"))
}

fn progress(dir: &Path, bytes_done: u64, size: u64) {
    let _ = write_json(
        &dir.join("progress.json"),
        &json!({"bytes_done": bytes_done, "size": size}),
    );
}

fn cancel_requested(dir: &Path) -> bool {
    dir.join("cancel.json").exists()
}

/// Run `body` on a background thread for the transfer in `dir`, recording how it ended.
fn spawn_transfer(dir: PathBuf, body: impl FnOnce(&Path) -> Result<Value> + Send + 'static) {
    std::thread::spawn(move || {
        let outcome = body(&dir);
        let exit = match outcome {
            Ok(result) => json!({"state": "completed", "ended_at": now(), "result": result}),
            Err(_) if cancel_requested(&dir) => json!({"state": "cancelled", "ended_at": now()}),
            Err(error) => json!({"state": "failed", "ended_at": now(), "error": error.to_string()}),
        };
        let _ = write_json(&dir.join("exit.json"), &exit);
    });
}

fn start_transfer(
    context: &Context,
    scope: &Value,
    key: &str,
    kind: &str,
    spec: Value,
    mut record: Value,
) -> Result<(PathBuf, bool)> {
    let owner = text(scope, "owner_id")?;
    let id = job_id(owner, key);
    record["job_id"] = json!(id);
    record["kind"] = json!(kind);
    record["owner_id"] = json!(owner);
    record["run_id"] = scope["run_id"].clone();
    record["digest"] = json!(hex::encode(Sha256::digest(serde_json::to_vec(&spec)?)));
    record["created_at"] = json!(now());
    record["detach"] = json!(false);
    let (dir, created) = reserve(context, &id, &record)?;
    if created {
        transfer_process(&dir)?;
    }
    Ok((dir, created))
}

/// `remote.fetch_blob {blob_id, path, idempotency_key}`: copy a channel attachment to `path`,
/// in the background. The broker decides, chunk by chunk with the run's own credential, whether
/// this run may read the attachment; the copy lands only whole and only with the attachment's
/// SHA-256, and never replaces a file that is there.
fn fetch_blob(
    context: &Context,
    root: &Path,
    params: &Value,
    scope: &Value,
    link: &BrokerLink,
) -> Result<Value> {
    let blob_id = text(params, "blob_id")?;
    ensure!(
        !blob_id.is_empty()
            && blob_id.len() <= 64
            && blob_id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'),
        "invalid_params: invalid blob_id"
    );
    let key = idempotency_key(params)?;
    let target = resolve_path(&context.home, root, text(params, "path")?)?;
    let spec = json!({"kind": "fetch_blob", "blob_id": blob_id, "path": target});
    let owner = text(scope, "owner_id")?;
    // A retry of a copy already made is answered with it, not refused for its finished file.
    if !context.jobs.join(job_id(owner, key)).exists() {
        refuse_protected_write(context, &target)?;
        ensure!(
            fs::symlink_metadata(&target).is_err(),
            "conflict: {} already exists; choose a new path",
            target.display()
        );
    }
    let record = json!({"path": target, "blob_id": blob_id});
    let (dir, created) = start_transfer(context, scope, key, "fetch_blob", spec, record)?;
    if created {
        let link = link.clone();
        let blob_id = blob_id.to_owned();
        spawn_transfer(dir.clone(), move |dir| {
            run_fetch(dir, &link, &blob_id, &target)
        });
    }
    job_view(&dir, 0)
}

fn run_fetch(dir: &Path, link: &BrokerLink, blob_id: &str, target: &Path) -> Result<Value> {
    let (mut stream, mut reader) = (link.connect)()?;
    let parent = target
        .parent()
        .ok_or_else(|| anyhow!("invalid_params: path needs a directory"))?;
    fs::create_dir_all(parent).map_err(|e| anyhow!("remote path unavailable: {e}"))?;
    let name = target
        .file_name()
        .ok_or_else(|| anyhow!("invalid_params: path needs a file name"))?
        .to_string_lossy();
    let temporary = parent.join(format!(
        ".{name}.crew-{}.part",
        uuid::Uuid::new_v4().simple()
    ));
    let copied = (|| -> Result<Value> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temporary)?;
        let mut hasher = Sha256::new();
        let mut sequence = 0u64;
        let mut offset = 0u64;
        let mut reported = 0u64;
        let (size, sha256) = loop {
            ensure!(!cancel_requested(dir), "cancelled: The copy was cancelled.");
            let answer = call(
                &mut stream,
                &mut reader,
                &link.credential,
                &mut sequence,
                "blob.read",
                json!({"blob_id": blob_id, "offset": offset}),
            )?;
            let size = answer["blob"]["size"]
                .as_u64()
                .ok_or_else(|| anyhow!("invalid_response: attachment size missing"))?;
            let sha256 = text(&answer["blob"], "sha256")?.to_ascii_lowercase();
            ensure!(
                size <= MAX_TRANSFER,
                "quota_exceeded: attachments over 1 GiB are not copied"
            );
            ensure!(
                answer["offset"].as_u64() == Some(offset),
                "invalid_response: attachment chunk out of order"
            );
            let bytes = hex::decode(text(&answer, "data_hex")?)?;
            file.write_all(&bytes)?;
            hasher.update(&bytes);
            offset += bytes.len() as u64;
            if offset - reported >= PROGRESS_EVERY || offset == size {
                progress(dir, offset, size);
                reported = offset;
            }
            if answer["complete"] == true || offset >= size {
                break (size, sha256);
            }
            ensure!(
                !bytes.is_empty(),
                "invalid_response: empty attachment chunk"
            );
        };
        ensure!(
            offset == size && hex::encode(hasher.finalize()) == sha256,
            "digest_mismatch: the copy does not match the attachment's SHA-256"
        );
        file.sync_all()?;
        drop(file);
        rename_new(&temporary, target)?;
        File::open(parent)?.sync_all()?;
        Ok(json!({"path": target, "size": size, "sha256": sha256}))
    })();
    if copied.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    copied
}

/// Rename `from` to `to` only if `to` does not exist.
fn rename_new(from: &Path, to: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let source = std::ffi::CString::new(from.as_os_str().as_encoded_bytes())?;
        let target = std::ffi::CString::new(to.as_os_str().as_encoded_bytes())?;
        let renamed = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                target.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if renamed == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            bail!(
                "conflict: {} already exists; choose a new path",
                to.display()
            );
        }
        // A filesystem without RENAME_NOREPLACE (some network filesystems) falls through.
        if !matches!(error.raw_os_error(), Some(libc::EINVAL | libc::ENOSYS)) {
            return Err(error.into());
        }
    }
    ensure!(
        fs::symlink_metadata(to).is_err(),
        "conflict: {} already exists; choose a new path",
        to.display()
    );
    fs::rename(from, to)?;
    Ok(())
}

/// A media type for a file name's extension, for how the channel shows the file.
fn media_type_for(name: &str) -> &'static str {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        _ => "application/octet-stream",
    }
}

/// `remote.share_file {path, name?, media_type?, idempotency_key}`: upload a server file of at
/// most 1 GiB to the run's channel as an attachment, in the background. The job's result names
/// the attachment (`blob_id`), which the daemon then posts. Upload rules (quotas, the run's
/// channel, Restricted for a remote run) are the broker's, as for every worker upload.
fn share_file(
    context: &Context,
    root: &Path,
    params: &Value,
    scope: &Value,
    link: &BrokerLink,
) -> Result<Value> {
    let key = idempotency_key(params)?;
    let path = resolve_path(&context.home, root, text(params, "path")?)?;
    let channel = text(scope, "channel_id")
        .map_err(|_| anyhow!("unsupported: This workspace server is too old to share server files; ask the host to update Crew."))?
        .to_owned();
    let metadata = fs::metadata(&path).map_err(|e| anyhow!("remote path unavailable: {e}"))?;
    ensure!(metadata.is_file(), "not a regular file");
    ensure!(
        metadata.len() <= MAX_TRANSFER,
        "quota_exceeded: maximum attachment is 1 GiB"
    );
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = match params.get("name") {
        None | Some(Value::Null) => file_name,
        Some(Value::String(name)) => name.clone(),
        Some(_) => bail!("invalid_params: name must be a string"),
    };
    let media_type = match params.get("media_type") {
        None | Some(Value::Null) => media_type_for(&name).to_owned(),
        Some(Value::String(media_type)) => media_type.clone(),
        Some(_) => bail!("invalid_params: media_type must be a string"),
    };
    let spec = json!({"kind": "share_file", "path": path, "name": name, "media_type": media_type});
    let record = json!({"path": path, "name": name, "media_type": media_type});
    let (dir, created) = start_transfer(context, scope, key, "share_file", spec, record)?;
    if created {
        let link = link.clone();
        let job = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .chars()
            .take(16)
            .collect::<String>();
        spawn_transfer(dir.clone(), move |dir| {
            run_share(dir, &link, &job, &channel, &path, &name, &media_type)
        });
    }
    job_view(&dir, 0)
}

fn run_share(
    dir: &Path,
    link: &BrokerLink,
    job: &str,
    channel: &str,
    path: &Path,
    name: &str,
    media_type: &str,
) -> Result<Value> {
    let mut file = File::open(path).map_err(|e| anyhow!("remote path unavailable: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; TRANSFER_CHUNK];
    let mut size = 0u64;
    loop {
        ensure!(
            !cancel_requested(dir),
            "cancelled: The upload was cancelled."
        );
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        ensure!(
            size <= MAX_TRANSFER,
            "quota_exceeded: maximum attachment is 1 GiB"
        );
        hasher.update(&buffer[..n]);
    }
    let sha256 = hex::encode(hasher.finalize());
    progress(dir, 0, size);
    let (mut stream, mut reader) = (link.connect)()?;
    let mut sequence = 0u64;
    let blob = call(
        &mut stream,
        &mut reader,
        &link.credential,
        &mut sequence,
        "blob.begin",
        json!({"channel_id": channel, "size": size, "sha256": sha256, "name": name, "media_type": media_type, "idempotency_key": format!("share-{job}-begin")}),
    )?;
    let blob_id = text(&blob, "id")?.to_owned();
    file.seek(SeekFrom::Start(0))?;
    let mut offset = 0u64;
    let mut reported = 0u64;
    while offset < size {
        ensure!(
            !cancel_requested(dir),
            "cancelled: The upload was cancelled."
        );
        let want = TRANSFER_CHUNK.min((size - offset) as usize);
        file.read_exact(&mut buffer[..want])?;
        call(
            &mut stream,
            &mut reader,
            &link.credential,
            &mut sequence,
            "blob.chunk",
            json!({"blob_id": blob_id, "offset": offset, "data_hex": hex::encode(&buffer[..want]), "idempotency_key": format!("share-{job}-{offset}")}),
        )?;
        offset += want as u64;
        if offset - reported >= PROGRESS_EVERY || offset == size {
            progress(dir, offset, size);
            reported = offset;
        }
    }
    call(
        &mut stream,
        &mut reader,
        &link.credential,
        &mut sequence,
        "blob.finish",
        json!({"blob_id": blob_id, "idempotency_key": format!("share-{job}-finish")}),
    )?;
    Ok(
        json!({"blob_id": blob_id, "name": name, "size": size, "sha256": sha256, "media_type": media_type}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "crew-full-{label}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&path).unwrap();
        fs::canonicalize(&path).unwrap()
    }

    fn context(home: &Path) -> Context {
        Context {
            home: home.to_path_buf(),
            jobs: home.join(".local/state/biorouter-crew/jobs"),
            executable: home.join(".local/bin/biorouter-crew"),
            program: home.join(".local/bin/biorouter-crew"),
        }
    }

    #[test]
    fn paths_resolve_from_home_absolute_or_the_work_folder() {
        let home = Path::new("/home/bob");
        let root = Path::new("/home/bob/crew-work/lab");
        assert_eq!(
            resolve_path(home, root, "~").unwrap(),
            PathBuf::from("/home/bob")
        );
        assert_eq!(
            resolve_path(home, root, "~/data/x.csv").unwrap(),
            PathBuf::from("/home/bob/data/x.csv")
        );
        assert_eq!(
            resolve_path(home, root, "/pool1/data/datasets").unwrap(),
            PathBuf::from("/pool1/data/datasets")
        );
        assert_eq!(
            resolve_path(home, root, "results/out.txt").unwrap(),
            PathBuf::from("/home/bob/crew-work/lab/results/out.txt")
        );
        assert_eq!(resolve_path(home, root, ".").unwrap(), root.join("."));
        assert!(resolve_path(home, root, "~alice/x").is_err());
        assert!(resolve_path(home, root, "a\0b").is_err());
    }

    #[test]
    fn job_ids_are_keyed_by_owner_not_run() {
        let one = job_id("owner-a", "key-1");
        assert_eq!(one, job_id("owner-a", "key-1"));
        assert_ne!(one, job_id("owner-b", "key-1"));
        assert_ne!(one, job_id("owner-a", "key-2"));
        assert_eq!(one.len(), 64);
        assert_eq!(one, hex::encode(Sha256::digest(b"owner-a:key-1")));
    }

    #[test]
    fn root_never_gets_full_access() {
        assert!(refuse_root(0)
            .unwrap_err()
            .to_string()
            .starts_with("forbidden:"));
        refuse_root(1000).unwrap();
    }

    #[test]
    fn write_modes_truncate_append_and_create_new() {
        let home = temp("write");
        let context = context(&home);
        let file = home.join("notes/a.txt");
        let write_with = |params: Value| write(&context, &file, &params);
        assert!(write_with(json!({"text": "x"})).is_err(), "no parents yet");
        let answer = write_with(json!({"text": "one\n", "create_parents": true})).unwrap();
        assert_eq!(answer["file_size"], 4);
        write_with(json!({"text": "two\n", "mode": "append"})).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "one\ntwo\n");
        write_with(json!({"data_hex": hex::encode("new\n")})).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "new\n");
        let refused = write_with(json!({"text": "again", "mode": "create_new"})).unwrap_err();
        assert!(refused.to_string().starts_with("conflict:"), "{refused}");
        assert!(write_with(json!({"text": "a", "data_hex": "61"})).is_err());
        assert!(write_with(json!({"text": "a", "mode": "sideways"})).is_err());
        let large = "a".repeat(MAX_CHUNK_BYTES + 1);
        assert!(write_with(json!({"text": large})).is_err());
        let _ = fs::remove_dir_all(&home);
    }

    /// The bridge executable and `authorized_keys` are refused however they are reached: by
    /// name, through `..`, or through a symlink.
    #[test]
    fn writes_to_the_bridge_and_authorized_keys_are_refused() {
        let home = temp("protect");
        let context = context(&home);
        fs::create_dir_all(home.join(".ssh")).unwrap();
        fs::create_dir_all(home.join(".local/bin")).unwrap();
        fs::write(&context.executable, b"bridge").unwrap();
        for target in [
            home.join(".ssh/authorized_keys"),
            home.join(".ssh/authorized_keys2"),
            home.join(".ssh/../.ssh/authorized_keys"),
            context.executable.clone(),
        ] {
            let error = write(&context, &target, &json!({"text": "x"})).unwrap_err();
            assert!(
                error.to_string().starts_with("forbidden:"),
                "{}: {error}",
                target.display()
            );
        }
        std::os::unix::fs::symlink(&context.executable, home.join("link-to-bridge")).unwrap();
        assert!(write(
            &context,
            &home.join("link-to-bridge"),
            &json!({"text": "x"})
        )
        .is_err());
        std::os::unix::fs::symlink(home.join("nowhere"), home.join("dangling")).unwrap();
        assert!(write(&context, &home.join("dangling"), &json!({"text": "x"})).is_err());
        assert_eq!(fs::read(&context.executable).unwrap(), b"bridge");
        // Other files in the same folders are ordinary.
        write(
            &context,
            &home.join(".ssh/config-notes"),
            &json!({"text": "x"}),
        )
        .unwrap();
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn reads_take_offsets_and_stay_within_a_frame() {
        let home = temp("read");
        let file = home.join("data.bin");
        fs::write(&file, b"hello world").unwrap();
        let answer = read(&file, &json!({"offset": 6, "length": 5})).unwrap();
        assert_eq!(answer["text_utf8"], "world");
        assert_eq!(answer["eof"], true);
        assert_eq!(answer["file_size"], 11);
        assert!(read(&file, &json!({"offset": 12})).is_err());
        assert!(read(&file, &json!({"length": MAX_CHUNK_BYTES + 1})).is_err());
        let big = home.join("big.txt");
        fs::write(&big, "\u{1}".repeat(MAX_CHUNK_BYTES + 10)).unwrap();
        let answer = read(&big, &json!({"offset": 0})).unwrap();
        assert_eq!(answer["size"], MAX_CHUNK_BYTES);
        assert_eq!(answer["eof"], false);
        assert!(
            answer.get("text_utf8").is_none(),
            "escaped text would overflow the frame"
        );
        assert!(serde_json::to_vec(&answer).unwrap().len() < crate::MAX_FRAME);
        let _ = fs::remove_dir_all(&home);
    }

    /// A read without a range is the whole file, as `remote.attach` uploads it in one chunk: up
    /// to the broker's chunk size it comes back whole, and a larger file is refused rather than
    /// cut, with what to do instead.
    #[test]
    fn a_read_without_a_range_is_the_whole_file_or_refused() {
        let home = temp("read-whole");
        let fits = home.join("fits.csv");
        fs::write(&fits, vec![b'a'; crate::MAX_CHUNK]).unwrap();
        let answer = read(&fits, &json!({"path": "fits.csv"})).unwrap();
        assert_eq!(answer["size"], crate::MAX_CHUNK);
        assert_eq!(answer["file_size"], crate::MAX_CHUNK);
        assert_eq!(answer["eof"], true);
        let larger = home.join("larger.csv");
        fs::write(&larger, vec![b'a'; crate::MAX_CHUNK + 1]).unwrap();
        for params in [json!({}), json!({"offset": null, "length": null})] {
            let error = read(&larger, &params).unwrap_err().to_string();
            assert!(error.starts_with("invalid_params:"), "{error}");
            assert!(error.contains("remote.share_file"), "{error}");
        }
        let part = read(&larger, &json!({"length": crate::MAX_CHUNK})).unwrap();
        assert_eq!(part["eof"], false);
        assert_eq!(part["next_offset"], crate::MAX_CHUNK);
        let _ = fs::remove_dir_all(&home);
    }

    /// The bridge's file is protected however `/proc/self/exe` names it, and the installed copy
    /// is protected even when the bridge runs from elsewhere; helpers start from the running
    /// image, which outlives a replaced file.
    #[test]
    fn the_bridge_finds_itself_after_a_newer_copy_replaced_it() {
        assert_eq!(
            without_deleted(PathBuf::from("/home/u/.local/bin/biorouter-crew (deleted)")),
            PathBuf::from("/home/u/.local/bin/biorouter-crew")
        );
        assert_eq!(
            without_deleted(PathBuf::from("/opt/crew/biorouter-crew")),
            PathBuf::from("/opt/crew/biorouter-crew")
        );
        let home = temp("installed");
        let mut context = context(&home);
        context.executable = PathBuf::from("/opt/crew/biorouter-crew");
        fs::create_dir_all(home.join(".local/bin")).unwrap();
        let error = write(
            &context,
            &home.join(".local/bin/biorouter-crew"),
            &json!({"text": "x"}),
        )
        .unwrap_err();
        assert!(error.to_string().starts_with("forbidden:"), "{error}");
        let this = Context::of_this_process().unwrap();
        assert!(!this.executable.to_string_lossy().ends_with(" (deleted)"));
        if cfg!(target_os = "linux") {
            assert_eq!(this.program, PathBuf::from("/proc/self/exe"));
        } else {
            assert_eq!(this.program, this.executable);
        }
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn listing_and_stat_follow_symlinks_and_report_missing_paths() {
        let home = temp("list");
        fs::write(home.join("a.txt"), b"a").unwrap();
        fs::create_dir(home.join("sub")).unwrap();
        std::os::unix::fs::symlink(home.join("sub"), home.join("link")).unwrap();
        let listing = list(&home, &json!({})).unwrap();
        let names: Vec<&str> = listing["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["a.txt", "link", "sub"]);
        assert_eq!(listing["entries"][1]["target_type"], "directory");
        assert_eq!(
            list(&home, &json!({"limit": 1})).unwrap()["truncated"],
            true
        );
        assert_eq!(stat(&home.join("link")).unwrap()["type"], "directory");
        assert_eq!(stat(&home.join("missing")).unwrap()["exists"], false);
        assert_eq!(
            hash(&home.join("a.txt")).unwrap()["sha256"],
            hex::encode(Sha256::digest(b"a"))
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn argv_runs_through_the_login_shell_exactly() {
        let argv = vec![
            "python3".to_owned(),
            "-c".to_owned(),
            "print('a b')".to_owned(),
        ];
        assert_eq!(
            job_argv(Path::new("/bin/bash"), None, Some(&argv)),
            vec![
                "/bin/bash",
                "-lc",
                "exec \"$@\"",
                "biorouter-crew-job",
                "python3",
                "-c",
                "print('a b')"
            ]
        );
        assert_eq!(
            job_argv(Path::new("/usr/bin/fish"), None, Some(&argv))[0],
            "/bin/sh"
        );
        assert_eq!(
            job_argv(Path::new("/bin/zsh"), Some("echo hi"), None),
            vec!["/bin/zsh", "-lc", "echo hi"]
        );
        assert_eq!(
            job_argv(Path::new("/bin/tcsh"), Some("echo hi"), None)[1],
            "-c"
        );
    }

    /// A folder-mode scope (or one from an older broker, with no `access`) never reaches the
    /// full-access code, and a public run never does either.
    #[test]
    fn full_access_needs_a_full_scope() {
        let home = temp("scope");
        let context = context(&home);
        let link = BrokerLink {
            connect: Arc::new(|| bail!("no broker in this test")),
            credential: "credential".into(),
        };
        let params = json!({"command": "true", "idempotency_key": "k"});
        for scope in [
            json!({"root": home, "public_provider": false, "owner_id": "o", "allow_execute": true, "expires_at": u64::MAX}),
            json!({"root": home, "public_provider": false, "owner_id": "o", "allow_execute": true, "expires_at": u64::MAX, "access": "folder"}),
            json!({"root": home, "public_provider": true, "owner_id": "o", "allow_execute": true, "expires_at": u64::MAX, "access": "full"}),
            json!({"root": home, "public_provider": false, "allow_execute": true, "expires_at": u64::MAX, "access": "full"}),
        ] {
            let error = handle_in(&context, "remote.shell", &params, &scope, &link).unwrap_err();
            assert!(
                ["invalid_scope:", "forbidden:"]
                    .iter()
                    .any(|code| error.to_string().starts_with(code)),
                "{scope}: {error}"
            );
        }
        assert!(!context.jobs.exists(), "nothing was recorded");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn job_records_are_owner_only_and_keyed_by_digest() {
        let home = temp("reserve");
        let context = context(&home);
        let record = json!({"job_id": "x", "owner_id": "o", "digest": "d", "kind": "shell", "created_at": now()});
        let id = job_id("o", "k");
        let (dir, created) = reserve(&context, &id, &record).unwrap();
        assert!(created);
        assert_eq!(fs::metadata(&context.jobs).unwrap().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
        assert_eq!(
            fs::metadata(dir.join("job.json")).unwrap().mode() & 0o777,
            0o600
        );
        let (again, created) = reserve(&context, &id, &record).unwrap();
        assert!(!created);
        assert_eq!(again, dir);
        let mut other = record.clone();
        other["digest"] = json!("e");
        assert!(reserve(&context, &id, &other)
            .unwrap_err()
            .to_string()
            .starts_with("conflict:"));
        let mut stranger = record;
        stranger["owner_id"] = json!("p");
        assert!(reserve(&context, &id, &stranger).is_err());
        let found = job_dir(&context, &json!({"job_id": id}), "o").unwrap();
        assert_eq!(found, dir);
        assert!(job_dir(&context, &json!({"job_id": id}), "p").is_err());
        assert!(
            job_dir(&context, &json!({"job_id": job_id("o", "none")}), "o")
                .unwrap_err()
                .to_string()
                .starts_with("not_found:")
        );
        let _ = fs::remove_dir_all(&home);
    }

    /// Eight running jobs refuse a ninth; at the record cap the oldest finished one goes.
    #[test]
    fn running_jobs_and_records_are_capped() {
        let home = temp("caps");
        let context = context(&home);
        let jobs = jobs_directory(&context).unwrap().to_path_buf();
        let make = |index: usize, finished: bool| {
            let dir = jobs.join(job_id("o", &format!("k{index}")));
            fs::create_dir(&dir).unwrap();
            write_json(
                &dir.join("job.json"),
                &json!({"owner_id": "o", "kind": "shell", "created_at": now()}),
            )
            .unwrap();
            if finished {
                write_json(
                    &dir.join("exit.json"),
                    &json!({"state": "completed", "ended_at": index}),
                )
                .unwrap();
            }
            dir
        };
        for index in 0..MAX_RUNNING {
            make(index, false);
        }
        assert!(make_room(&jobs, false)
            .unwrap_err()
            .to_string()
            .starts_with("quota_exceeded:"));
        for dir in fs::read_dir(&jobs).unwrap() {
            fs::remove_dir_all(dir.unwrap().path()).unwrap();
        }
        let oldest = make(0, true);
        for index in 1..MAX_RECORDS {
            make(index, true);
        }
        make_room(&jobs, false).unwrap();
        assert!(!oldest.exists(), "the oldest finished record made room");
        assert_eq!(fs::read_dir(&jobs).unwrap().count(), MAX_RECORDS - 1);
        let _ = fs::remove_dir_all(&home);
    }

    /// Each transfer holds a broker connection, and the broker takes 8 per account: at most
    /// four transfers run at once, while commands may still start.
    #[test]
    fn running_transfers_are_capped_below_the_brokers_connections() {
        let home = temp("transfer-caps");
        let context = context(&home);
        let jobs = jobs_directory(&context).unwrap().to_path_buf();
        for (index, kind) in ["fetch_blob", "share_file", "fetch_blob", "share_file"]
            .into_iter()
            .enumerate()
        {
            let dir = jobs.join(job_id("o", &format!("t{index}")));
            fs::create_dir(&dir).unwrap();
            write_json(
                &dir.join("job.json"),
                &json!({"owner_id": "o", "kind": kind, "created_at": now()}),
            )
            .unwrap();
        }
        let error = make_room(&jobs, true).unwrap_err().to_string();
        assert!(error.starts_with("quota_exceeded:"), "{error}");
        assert!(error.contains("file copies"), "{error}");
        make_room(&jobs, false).unwrap();
        let record = json!({"job_id": "x", "owner_id": "o", "digest": "d", "kind": "share_file", "created_at": now()});
        assert!(reserve(&context, &job_id("o", "fifth"), &record).is_err());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn media_types_follow_the_extension() {
        assert_eq!(media_type_for("result.CSV"), "text/csv");
        assert_eq!(media_type_for("plot.png"), "image/png");
        assert_eq!(media_type_for("model"), "application/octet-stream");
    }
}
