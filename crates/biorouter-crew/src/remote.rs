//! Own-UID scoped file operations and fail-closed, single-process Linux jobs.
use anyhow::{anyhow, bail, ensure, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

const MAX_FILE: usize = 262_144;
const MAX_OUTPUT: usize = 65_536;
struct Job {
    child: Option<Child>,
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    status: String,
    exit_code: Option<i32>,
    run_id: String,
    digest: String,
    record: PathBuf,
    readers: Vec<std::thread::JoinHandle<()>>,
}
static JOBS: LazyLock<Mutex<HashMap<String, Arc<Mutex<Job>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .ok_or_else(|| anyhow!("invalid_params: {key} must be a string"))
}
/// Folders under `HOME` a remote work folder may never equal, sit inside, or contain:
/// authentication and Crew state, and (D15, SR4) the places a program or shell startup file
/// the SSH login runs lives, so a confined worker writing into its work folder can never replace
/// the bridge binary or wrap it through shell startup and become the in-path process of the
/// join's threat model (M2).
const PROTECTED_UNDER_HOME: [&str; 9] = [
    ".ssh",
    ".aws",
    ".config",
    ".local/state",
    ".local/share/biorouter-crew",
    ".local/bin",
    "bin",
    ".bashrc.d",
    ".profile.d",
];

/// Every protected folder for one spelling of `HOME` and one location of the bridge: the
/// fixed list under `HOME`, the directory holding the bridge executable, and that directory's
/// ancestors strictly inside `HOME` (so nothing between `HOME` and the bridge can be granted
/// either). A bridge outside `HOME` protects only its own directory; its ancestors are refused
/// anyway, because a work folder may not contain a protected folder. A bridge directly in
/// `HOME` (or above it) adds nothing: a work folder can never be `HOME` or an ancestor of it,
/// and protecting everything *inside* `HOME` would refuse every work folder.
fn protected_folders(home: &Path, bridge: &Path) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = PROTECTED_UNDER_HOME
        .iter()
        .map(|folder| home.join(folder))
        .collect();
    if let Some(directory) = bridge
        .parent()
        .filter(|directory| !home.starts_with(directory))
    {
        folders.push(directory.to_path_buf());
        folders.extend(
            directory
                .ancestors()
                .skip(1)
                .take_while(|ancestor| *ancestor != home && ancestor.starts_with(home))
                .map(Path::to_path_buf),
        );
    }
    folders
}

/// The path-only rules for a remote work folder, given every spelling of `HOME` and every
/// resolved location of the bridge executable: an absolute, dedicated directory (at least two
/// levels deep, not `HOME` or an ancestor of it) that is not equal to, inside, or an ancestor of
/// any protected folder.
fn check_work_folder(root: &Path, homes: &[PathBuf], bridges: &[PathBuf]) -> Result<()> {
    ensure!(root.is_absolute(), "invalid_scope: root must be absolute");
    ensure!(
        root.components().count() >= 3 && homes.iter().all(|home| !home.starts_with(root)),
        "invalid_scope: choose a dedicated work directory, not HOME or a filesystem root"
    );
    for home in homes {
        for bridge in bridges {
            for protected in protected_folders(home, bridge) {
                ensure!(
                    !root.starts_with(&protected) && !protected.starts_with(root),
                    "invalid_scope: remote work directory overlaps protected authentication, application state, program or shell startup folders; choose a folder such as ~/crew-work/<workspace>"
                );
            }
        }
    }
    Ok(())
}

/// `path` as given and, when it differs, as the filesystem resolves it, so a `HOME` reached
/// through a symlink protects the same folders as its real location.
fn path_variants(path: &Path) -> Vec<PathBuf> {
    let mut variants = vec![path.to_path_buf()];
    if let Ok(resolved) = std::fs::canonicalize(path) {
        if resolved != path {
            variants.push(resolved);
        }
    }
    variants
}

/// Where the running bridge executable is, as reported and as resolved. Fails closed: a bridge
/// that cannot locate itself cannot protect itself.
fn bridge_executables() -> Result<Vec<PathBuf>> {
    let executable = std::env::current_exe()
        .map_err(|_| anyhow!("invalid_scope: cannot locate the Crew bridge executable"))?;
    Ok(path_variants(&executable))
}

fn root(scope: &Value) -> Result<PathBuf> {
    ensure!(
        scope["public_provider"] == false,
        "forbidden: public providers cannot access remote files or jobs"
    );
    let root = PathBuf::from(text(scope, "root")?);
    ensure!(root.is_absolute(), "invalid_scope: root must be absolute");
    let canonical = std::fs::canonicalize(&root)?;
    ensure!(
        canonical == root,
        "invalid_scope: root must be canonical without symlinks"
    );
    let home = PathBuf::from(std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME unavailable"))?);
    check_work_folder(&root, &path_variants(&home), &bridge_executables()?)?;
    use std::os::unix::fs::MetadataExt;
    ensure!(
        std::fs::metadata(&root)?.uid() == unsafe { libc::geteuid() },
        "forbidden: work directory must belong to the SSH user"
    );
    let mut directories = vec![root.clone()];
    let mut inspected = 0usize;
    while let Some(directory) = directories.pop() {
        ensure!(
            !(directory.join("journal.jsonl").exists() && directory.join("writer.lock").exists()),
            "invalid_scope: work directory overlaps a Crew authority store"
        );
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            inspected += 1;
            ensure!(
                inspected <= 10000,
                "work directory exceeds bounded authority inspection limit"
            );
            if entry.file_type()?.is_dir() {
                directories.push(entry.path());
            }
        }
    }
    Ok(root)
}
fn relative(params: &Value) -> Result<PathBuf> {
    let path = PathBuf::from(params.get("path").and_then(Value::as_str).unwrap_or("."));
    ensure!(
        !path.is_absolute()
            && path
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir)),
        "invalid_params: path must remain relative to the approved work directory"
    );
    Ok(path)
}
fn open_scoped(root: &Path, path: &Path, write: bool) -> Result<File> {
    use std::os::unix::ffi::OsStrExt;
    use std::{
        ffi::CString,
        os::fd::{AsRawFd, FromRawFd},
    };
    let name = CString::new(root.as_os_str().as_bytes())?;
    let fd = unsafe {
        libc::open(
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        )
    };
    ensure!(fd >= 0, "approved directory unavailable");
    let mut dir = unsafe { File::from_raw_fd(fd) };
    let parts: Vec<_> = path
        .components()
        .filter_map(|c| {
            if let Component::Normal(v) = c {
                Some(v)
            } else {
                None
            }
        })
        .collect();
    if parts.is_empty() {
        ensure!(!write, "invalid_params: cannot write directory");
        return Ok(dir);
    }
    for (index, part) in parts.iter().enumerate() {
        let last = index + 1 == parts.len();
        let name = CString::new(part.as_bytes())?;
        let flags = libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if last {
                if write {
                    libc::O_WRONLY | libc::O_CREAT
                } else {
                    libc::O_RDONLY
                }
            } else {
                libc::O_RDONLY | libc::O_DIRECTORY
            };
        let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags, 0o600) };
        ensure!(
            fd >= 0,
            "remote path unavailable: {}",
            std::io::Error::last_os_error()
        );
        dir = unsafe { File::from_raw_fd(fd) };
    }
    if dir.metadata()?.is_file() {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            dir.metadata()?.nlink() == 1,
            "hard-linked files are not admitted into remote scope"
        );
    }
    if write {
        ensure!(dir.metadata()?.is_file(), "not a regular file");
        dir.set_len(0)?;
    }
    Ok(dir)
}
pub fn handle(method: &str, params: &Value, scope: &Value) -> Result<Value> {
    ensure!(
        cfg!(target_os = "linux"),
        "unsupported: scoped remote operations require Linux"
    );
    let root = root(scope)?;
    let path = relative(params)?;
    match method {
        "remote.list" => list_files(&root, &path),
        "remote.read" | "remote.hash" => read_file(&root, &path, method),
        "remote.write" => write_file(&root, &path, params),
        "remote.execute" => start_job(&root, params, scope),
        "remote.job_status" | "remote.cancel" => job_status(method, params, scope),
        _ => bail!("unsupported: remote operation unavailable"),
    }
}
fn list_files(root: &Path, path: &Path) -> Result<Value> {
    let directory = open_scoped(root, path, false)?;
    ensure!(directory.metadata()?.is_dir(), "not a directory");
    use std::os::fd::AsRawFd;
    let entries = std::fs::read_dir(format!("/proc/self/fd/{}", directory.as_raw_fd()))?;
    let mut result = Vec::new();
    for entry in entries.take(501) {
        ensure!(
            result.len() < 500,
            "directory contains more than 500 entries; select a narrower directory"
        );
        let entry = entry?;
        let kind = entry.file_type()?;
        result.push(json!({"name":entry.file_name().to_string_lossy(),"directory":kind.is_dir(),"symlink":kind.is_symlink()}));
    }
    Ok(json!({"path":path,"entries":result}))
}

fn read_file(root: &Path, path: &Path, method: &str) -> Result<Value> {
    let mut file = open_scoped(root, path, false)?;
    ensure!(file.metadata()?.is_file(), "not a regular file");
    ensure!(
        file.metadata()?.len() <= MAX_FILE as u64,
        "file exceeds bounded read limit (256 KiB)"
    );
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_FILE as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_FILE, "file exceeds limit");
    let hash = hex::encode(Sha256::digest(&bytes));
    if method == "remote.hash" {
        Ok(json!({"sha256":hash,"size":bytes.len()}))
    } else {
        Ok(
            json!({"path":path,"data_hex":hex::encode(&bytes),"text_utf8":std::str::from_utf8(&bytes).ok().filter(|_|bytes.len()<=65536),"sha256":hash,"size":bytes.len()}),
        )
    }
}

fn write_file(root: &Path, path: &Path, params: &Value) -> Result<Value> {
    ensure!(
        params.get("text").is_some() != params.get("data_hex").is_some(),
        "provide exactly one of text or data_hex"
    );
    let bytes = if let Some(text) = params.get("text").and_then(Value::as_str) {
        text.as_bytes().to_vec()
    } else {
        hex::decode(text(params, "data_hex")?)?
    };
    ensure!(bytes.len() <= MAX_FILE, "write exceeds 256 KiB limit");
    let mut file = open_scoped(root, path, true)?;
    ensure!(file.metadata()?.is_file(), "not a regular file");
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(json!({"path":path,"size":bytes.len(),"sha256":hex::encode(Sha256::digest(bytes))}))
}

fn job_status(method: &str, params: &Value, scope: &Value) -> Result<Value> {
    let id = text(params, "job_id")?;
    let jobs = JOBS.lock().map_err(|_| anyhow!("job state unavailable"))?;
    let Some(job) = jobs.get(id).cloned() else {
        ensure!(
            id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid job ID"
        );
        drop(jobs);
        let state =
            PathBuf::from(std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME unavailable"))?)
                .join(".local/state/biorouter-crew/remote-jobs")
                .join(id);
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(state)?;
        let mut data = Vec::new();
        file.take(1_048_577).read_to_end(&mut data)?;
        ensure!(data.len() <= 1_048_576, "invalid job record");
        let mut record: Value = serde_json::from_slice(&data)?;
        ensure!(
            record["run_id"].as_str() == Some(text(scope, "run_id")?),
            "forbidden: job belongs to another run"
        );
        if matches!(record["status"].as_str(), Some("starting" | "running")) {
            record["status"] = json!("unknown_after_disconnect");
            record["message"] =
                json!("Inspect output files before retrying; no process is adopted by PID.");
        }
        return Ok(record);
    };
    drop(jobs);
    let mut job = job.lock().map_err(|_| anyhow!("job state unavailable"))?;
    ensure!(
        job.run_id == text(scope, "run_id")?,
        "forbidden: job belongs to another run"
    );
    if method == "remote.cancel" {
        stop_job(&mut job, "cancelled");
    }
    snapshot(id, &job)
}

fn snapshot(id: &str, job: &Job) -> Result<Value> {
    Ok(
        json!({"job_id":id,"status":job.status,"exit_code":job.exit_code,"stdout":String::from_utf8_lossy(&job.stdout.lock().map_err(|_|anyhow!("output unavailable"))?),"stderr":String::from_utf8_lossy(&job.stderr.lock().map_err(|_|anyhow!("output unavailable"))?)}),
    )
}
fn persist_job(job: &mut Job) {
    for reader in job.readers.drain(..) {
        let _ = reader.join();
    }
    if let Ok(mut result) = snapshot(
        job.record
            .file_name()
            .and_then(|p| p.to_str())
            .unwrap_or(""),
        job,
    ) {
        result["run_id"] = json!(job.run_id);
        result["digest"] = json!(job.digest);
        let persisted = (|| -> Result<()> {
            let parent = job
                .record
                .parent()
                .ok_or_else(|| anyhow!("job state directory missing"))?;
            let temporary = parent.join(format!(".job-{}.tmp", uuid::Uuid::new_v4()));
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(&serde_json::to_vec(&result)?)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, &job.record)?;
            File::open(parent)?.sync_all()?;
            Ok(())
        })();
        if persisted.is_err() {
            job.status = "outcome_not_durable".into();
        }
    }
}
fn stop_job(job: &mut Job, status: &str) {
    if let Some(child) = job.child.as_mut() {
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        };
        let _ = child.kill();
        let _ = child.wait();
        job.child = None;
        job.status = status.into();
        persist_job(job);
    }
}
fn pump(
    mut stream: impl Read + Send + 'static,
    output: Arc<Mutex<Vec<u8>>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            let n = match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if let Ok(mut out) = output.lock() {
                let remaining = MAX_OUTPUT.saturating_sub(out.len());
                out.extend_from_slice(&buffer[..n.min(remaining)]);
            }
        }
    })
}
fn validate_work_tree(root: &Path) -> Result<()> {
    let mut directories = vec![root.to_path_buf()];
    let mut count = 0usize;
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            count += 1;
            ensure!(count <= 10000, "execution work tree exceeds 10000 entries");
            let metadata = entry.path().symlink_metadata()?;
            ensure!(
                !metadata.file_type().is_symlink(),
                "execution work tree contains a symlink; use a dedicated regular-file directory"
            );
            use std::os::unix::fs::MetadataExt;
            ensure!(
                !metadata.is_file() || metadata.nlink() == 1,
                "execution work tree contains a hard-linked file"
            );
            if metadata.is_dir() {
                directories.push(entry.path());
            }
        }
    }
    Ok(())
}
fn reserve_job_record(root: &Path, id: &str, digest: &str, run: &str) -> Result<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME unavailable"))?);
    let state = home.join(".local/state/biorouter-crew/remote-jobs");
    ensure!(
        !state.starts_with(root),
        "invalid_scope: work root contains Crew authority state"
    );
    std::fs::create_dir_all(&state)?;
    ensure!(
        !std::fs::symlink_metadata(&state)?.file_type().is_symlink(),
        "remote job state must not be a symlink"
    );
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700))?;
    let record = state.join(id);
    use std::os::unix::fs::OpenOptionsExt;
    let mut marker = std::fs::OpenOptions::new()
        .mode(0o600)
        .create_new(true)
        .write(true)
        .open(&record)
        .map_err(|e| {
            anyhow!("execution outcome may already exist; inspect before retrying: {e}")
        })?;
    marker.write_all(
        json!({"status":"starting","digest":digest,"run_id":run})
            .to_string()
            .as_bytes(),
    )?;
    marker.sync_all()?;
    // Persist the invocation name and newly created state hierarchy before execution.
    for directory in state.ancestors() {
        File::open(directory)?.sync_all()?;
        if directory == home {
            break;
        }
    }
    Ok(record)
}
fn spawn_job_child(root: &Path, argv: &[String], expiry: u64, deadline: Instant) -> Result<Child> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("__crew-exec")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", root)
        .env("TMPDIR", root)
        .env("LANG", "C.UTF-8")
        .env("OPENBLAS_NUM_THREADS", "1")
        .env("OMP_NUM_THREADS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(root);
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    ensure!(
        Instant::now() < deadline,
        "remote execution grant expired during admission"
    );
    let mut child = command.spawn()?;
    let input =
        json!({"root":root,"argv":argv,"parent_pid":std::process::id(),"expires_at":expiry});
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("child input missing"))?
        .write_all(serde_json::to_string(&input)?.as_bytes())?;
    Ok(child)
}
fn monitor_job(job: Arc<Mutex<Job>>, deadline: Instant) {
    let monitor = job.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(50));
        let Ok(mut job) = monitor.lock() else { break };
        let Some(child) = job.child.as_mut() else {
            break;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                job.exit_code = status.code();
                job.status = if status.success() {
                    "completed"
                } else {
                    "failed"
                }
                .into();
                job.child = None;
                persist_job(&mut job);
                break;
            }
            Err(_) => {
                stop_job(&mut job, "failed");
                break;
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            stop_job(&mut job, "timed_out");
            break;
        }
    });
}

fn start_job(root: &Path, params: &Value, scope: &Value) -> Result<Value> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let expiry = scope["expires_at"]
        .as_u64()
        .ok_or_else(|| anyhow!("grant expiry missing"))?;
    ensure!(expiry > now, "remote execution grant expired");
    let timeout = params
        .get("timeout_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(30)
        .clamp(1, 60)
        .min(expiry - now);
    let deadline = Instant::now() + Duration::from_secs(timeout);
    ensure!(
        scope["allow_execute"] == true,
        "forbidden: remote execution requires an explicit human grant"
    );
    validate_work_tree(root)?;
    let argv: Vec<String> = serde_json::from_value(params["argv"].clone())?;
    ensure!(
        !argv.is_empty()
            && argv.len() <= 64
            && argv.iter().all(|v| v.len() <= 16_384 && !v.contains('\0')),
        "invalid_params: bounded argv required"
    );
    let key = text(params, "idempotency_key")?;
    ensure!(key.len() <= 128, "idempotency key too long");
    let run = text(scope, "run_id")?;
    let id = hex::encode(Sha256::digest(format!("{run}:{key}").as_bytes()));
    let digest = hex::encode(Sha256::digest(serde_json::to_vec(params)?));
    let mut jobs = JOBS.lock().map_err(|_| anyhow!("job state unavailable"))?;
    if let Some(job) = jobs.get(&id) {
        let job = job.lock().map_err(|_| anyhow!("job state unavailable"))?;
        ensure!(job.digest == digest, "idempotency conflict");
        return snapshot(&id, &job);
    }
    ensure!(
        jobs.len() < 128,
        "job history capacity reached; reconnect after inspecting completed work"
    );
    ensure!(
        jobs.values()
            .filter(|j| j.lock().map(|j| j.child.is_some()).unwrap_or(true))
            .count()
            < 4,
        "remote job concurrency limit reached"
    );
    let record = reserve_job_record(root, &id, &digest, run)?;
    let mut child = spawn_job_child(root, &argv, expiry, deadline)?;
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let out_reader = pump(
        child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("child output missing"))?,
        stdout.clone(),
    );
    let err_reader = pump(
        child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("child output missing"))?,
        stderr.clone(),
    );
    let job = Arc::new(Mutex::new(Job {
        child: Some(child),
        stdout,
        stderr,
        status: "running".into(),
        exit_code: None,
        run_id: run.into(),
        digest,
        record,
        readers: vec![out_reader, err_reader],
    }));
    jobs.insert(id.clone(), job.clone());
    drop(jobs);
    monitor_job(job.clone(), deadline);
    let locked = job.lock().map_err(|_| anyhow!("job state unavailable"))?;
    let result = snapshot(&id, &locked)?;
    Ok(result)
}

/// Called only in a fresh helper process, before any agent-controlled executable.
pub fn exec_helper() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        return linux_exec();
    }
    #[cfg(not(target_os = "linux"))]
    {
        bail!("unsupported: Linux confinement required")
    }
}
#[cfg(target_os = "linux")]
fn linux_exec() -> Result<()> {
    use std::os::unix::process::CommandExt;
    let mut bytes = Vec::new();
    std::io::stdin().take(1_048_577).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1_048_576, "helper input exceeds limit");
    let input: Value = serde_json::from_slice(&bytes)?;
    let root = PathBuf::from(text(&input, "root")?);
    let argv: Vec<String> = serde_json::from_value(input["argv"].clone())?;
    ensure!(!argv.is_empty(), "argv missing");
    unsafe {
        ensure!(
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == 0,
            "parent lifecycle unavailable"
        );
        ensure!(
            libc::getppid() as u64 == input["parent_pid"].as_u64().unwrap_or(0),
            "bridge parent disappeared"
        );
        ensure!(
            libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) == 0,
            "no-new-privileges unavailable"
        );
    }
    for (resource, limit) in [
        (libc::RLIMIT_CPU, 60),
        (libc::RLIMIT_FSIZE, 16 * 1024 * 1024),
        (libc::RLIMIT_AS, 1024 * 1024 * 1024),
        (libc::RLIMIT_NOFILE, 64),
    ] {
        let limits = libc::rlimit {
            rlim_cur: limit,
            rlim_max: limit,
        };
        ensure!(
            unsafe { libc::setrlimit(resource, &limits) } == 0,
            "resource limit unavailable"
        );
    }
    confine(&root)?;
    ensure!(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs()
            < input["expires_at"].as_u64().unwrap_or(0),
        "remote execution grant expired before exec"
    );
    let error = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .exec();
    Err(error.into())
}
#[cfg(target_os = "linux")]
fn confine(root: &Path) -> Result<()> {
    use landlock::{
        Access, AccessFs, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset, RulesetAttr,
        RulesetCreatedAttr, RulesetStatus, ABI,
    };
    let abi = ABI::V3;
    let mut rules = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(abi))?
        .create()?;
    rules = rules.add_rule(PathBeneath::new(
        PathFd::new(root)?,
        AccessFs::from_all(abi),
    ))?;
    for path in [
        "/usr/bin",
        "/usr/lib",
        "/usr/lib64",
        "/usr/local/lib",
        "/bin",
        "/lib",
        "/lib64",
    ] {
        if Path::new(path).exists() {
            rules = rules.add_rule(PathBeneath::new(
                PathFd::new(path)?,
                AccessFs::from_read(abi),
            ))?;
        }
    }
    for path in ["/etc/ld.so.cache", "/etc/localtime", "/dev/urandom"] {
        if Path::new(path).exists() {
            rules = rules.add_rule(PathBeneath::new(PathFd::new(path)?, AccessFs::ReadFile))?;
        }
    }
    rules = rules.add_rule(PathBeneath::new(
        PathFd::new("/dev/null")?,
        AccessFs::ReadFile | AccessFs::WriteFile,
    ))?;
    ensure!(
        matches!(rules.restrict_self()?.ruleset, RulesetStatus::FullyEnforced),
        "unsupported: full Landlock read/write confinement required"
    );
    use seccompiler::{apply_filter, SeccompAction, SeccompFilter, TargetArch};
    let allowed = [
        libc::SYS_read,
        libc::SYS_write,
        libc::SYS_readv,
        libc::SYS_writev,
        libc::SYS_pread64,
        libc::SYS_pwrite64,
        libc::SYS_close,
        libc::SYS_lseek,
        libc::SYS_fstat,
        libc::SYS_newfstatat,
        libc::SYS_statx,
        libc::SYS_openat,
        libc::SYS_getdents64,
        libc::SYS_readlinkat,
        libc::SYS_faccessat,
        libc::SYS_faccessat2,
        libc::SYS_brk,
        libc::SYS_mmap,
        libc::SYS_mprotect,
        libc::SYS_munmap,
        libc::SYS_mremap,
        libc::SYS_madvise,
        libc::SYS_futex,
        libc::SYS_set_tid_address,
        libc::SYS_set_robust_list,
        libc::SYS_rseq,
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigreturn,
        libc::SYS_sigaltstack,
        libc::SYS_getpid,
        libc::SYS_getppid,
        libc::SYS_gettid,
        libc::SYS_getuid,
        libc::SYS_geteuid,
        libc::SYS_getgid,
        libc::SYS_getegid,
        libc::SYS_uname,
        libc::SYS_getcwd,
        libc::SYS_chdir,
        libc::SYS_fchdir,
        libc::SYS_mkdirat,
        libc::SYS_unlinkat,
        libc::SYS_renameat,
        libc::SYS_renameat2,
        libc::SYS_linkat,
        libc::SYS_symlinkat,
        libc::SYS_ftruncate,
        libc::SYS_truncate,
        libc::SYS_fsync,
        libc::SYS_fdatasync,
        libc::SYS_dup,
        libc::SYS_dup3,
        libc::SYS_getrandom,
        libc::SYS_clock_gettime,
        libc::SYS_gettimeofday,
        libc::SYS_nanosleep,
        libc::SYS_clock_nanosleep,
        libc::SYS_times,
        libc::SYS_getrusage,
        libc::SYS_sched_getaffinity,
        libc::SYS_execve,
        libc::SYS_exit,
        libc::SYS_exit_group,
    ];
    let mut filter: std::collections::BTreeMap<i64, Vec<seccompiler::SeccompRule>> =
        allowed.into_iter().map(|call| (call, Vec::new())).collect();
    #[cfg(target_arch = "x86_64")]
    {
        for call in [
            libc::SYS_arch_prctl,
            libc::SYS_open,
            libc::SYS_stat,
            libc::SYS_lstat,
            libc::SYS_access,
            libc::SYS_readlink,
            libc::SYS_dup2,
            libc::SYS_getrlimit,
            libc::SYS_mkdir,
            libc::SYS_rmdir,
            libc::SYS_unlink,
            libc::SYS_rename,
        ] {
            filter.insert(call, Vec::new());
        }
    }
    let fcntl_rules = [
        libc::F_GETFD,
        libc::F_SETFD,
        libc::F_GETFL,
        libc::F_SETFL,
        libc::F_DUPFD,
        libc::F_DUPFD_CLOEXEC,
    ]
    .into_iter()
    .map(|operation| {
        let condition = seccompiler::SeccompCondition::new(
            1,
            seccompiler::SeccompCmpArgLen::Dword,
            seccompiler::SeccompCmpOp::Eq,
            operation as u64,
        )?;
        seccompiler::SeccompRule::new(vec![condition])
    })
    .collect::<std::result::Result<Vec<_>, _>>()?;
    filter.insert(libc::SYS_fcntl, fcntl_rules);
    // CPython's script fopen sets close-on-exec with FIOCLEX and treats EPERM as fatal.
    let close_on_exec = seccompiler::SeccompCondition::new(
        1,
        seccompiler::SeccompCmpArgLen::Dword,
        seccompiler::SeccompCmpOp::Eq,
        libc::FIOCLEX as u64,
    )?;
    filter.insert(
        libc::SYS_ioctl,
        vec![seccompiler::SeccompRule::new(vec![close_on_exec])?],
    );
    let filter = SeccompFilter::new(
        filter,
        SeccompAction::Errno(libc::EPERM as u32),
        SeccompAction::Allow,
        TargetArch::try_from(std::env::consts::ARCH)
            .map_err(|e| anyhow!("unsupported architecture: {e}"))?,
    )?;
    let program: seccompiler::BpfProgram = filter.try_into()?;
    apply_filter(&program)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/bob";
    const BRIDGE: &str = "/home/bob/.local/bin/biorouter-crew";

    fn check(root: &str, bridge: &str) -> Result<()> {
        check_work_folder(
            Path::new(root),
            &[PathBuf::from(HOME)],
            &[PathBuf::from(bridge)],
        )
    }

    fn refused(root: &str, bridge: &str) {
        let error = check(root, bridge).expect_err(root);
        assert!(error.to_string().starts_with("invalid_scope:"), "{error}");
    }

    #[test]
    fn each_protected_folder_is_refused_equal_inside_or_as_an_ancestor() {
        for folder in PROTECTED_UNDER_HOME {
            let protected = format!("{HOME}/{folder}");
            refused(&protected, BRIDGE);
            refused(&format!("{protected}/work"), BRIDGE);
            refused(&format!("{protected}/deep/er"), BRIDGE);
        }
        // Ancestors of a protected folder contain it.
        for ancestor in [
            "/home/bob/.local",
            "/home/bob/.local/share",
            "/home/bob",
            "/home",
            "/",
        ] {
            refused(ancestor, BRIDGE);
        }
    }

    #[test]
    fn the_new_d15_folders_are_refused() {
        for root in [
            "/home/bob/.local/bin",
            "/home/bob/bin",
            "/home/bob/bin/tools",
            "/home/bob/.bashrc.d",
            "/home/bob/.profile.d",
            "/home/bob/.profile.d/conda",
        ] {
            refused(root, "/opt/crew/bin/biorouter-crew");
        }
    }

    #[test]
    fn the_bridge_directory_and_its_ancestors_up_to_home_are_refused() {
        let bridge = "/home/bob/tools/crew/bin/biorouter-crew";
        for root in [
            "/home/bob/tools/crew/bin",
            "/home/bob/tools/crew/bin/scratch",
            "/home/bob/tools/crew",
            "/home/bob/tools/crew/lib",
            "/home/bob/tools",
            "/home/bob/tools/other-project",
        ] {
            refused(root, bridge);
        }
        // Siblings of the chain under HOME stay available.
        check("/home/bob/crew-work/lab", bridge).unwrap();
        check("/home/bob/toolshed", bridge).unwrap();
    }

    #[test]
    fn a_bridge_outside_home_protects_its_own_directory() {
        let bridge = "/opt/crew/bin/biorouter-crew";
        refused("/opt/crew/bin", bridge);
        refused("/opt/crew/bin/work", bridge);
        refused("/opt/crew", bridge);
        check("/opt/crew/work", bridge).unwrap();
        check("/home/bob/crew-work/lab", bridge).unwrap();
    }

    #[test]
    fn a_bridge_directly_in_home_does_not_refuse_every_work_folder() {
        let bridge = "/home/bob/biorouter-crew";
        check("/home/bob/crew-work/lab", bridge).unwrap();
        refused("/home/bob", bridge);
    }

    #[test]
    fn a_dedicated_crew_work_folder_is_accepted() {
        check("/home/bob/crew-work/lab", BRIDGE).unwrap();
        check("/home/bob/crew-work", BRIDGE).unwrap();
        check("/home/bob/projects/analysis", BRIDGE).unwrap();
        check("/scratch/bob/lab", BRIDGE).unwrap();
        // Names that only share a prefix with a protected folder are different folders.
        check("/home/bob/binaries", BRIDGE).unwrap();
        check("/home/bob/.local-work", BRIDGE).unwrap();
    }

    #[test]
    fn every_spelling_of_home_and_the_bridge_is_protected() {
        let homes = [PathBuf::from("/home/bob"), PathBuf::from("/data/home/bob")];
        let bridges = [
            PathBuf::from("/home/bob/.local/bin/biorouter-crew"),
            PathBuf::from("/data/home/bob/apps/crew/biorouter-crew"),
        ];
        for root in [
            "/data/home/bob/.ssh",
            "/data/home/bob/.local/bin",
            "/data/home/bob/bin",
            "/data/home/bob/apps/crew",
            "/data/home/bob/apps",
            "/data/home/bob",
        ] {
            assert!(
                check_work_folder(Path::new(root), &homes, &bridges).is_err(),
                "{root}"
            );
        }
        check_work_folder(Path::new("/data/home/bob/crew-work/lab"), &homes, &bridges).unwrap();
    }

    #[test]
    fn relative_and_shallow_roots_are_refused() {
        refused("crew-work/lab", BRIDGE);
        refused("/srv", BRIDGE);
    }

    #[test]
    fn the_running_bridge_can_locate_itself() {
        let bridges = bridge_executables().unwrap();
        assert!(bridges.iter().all(|bridge| bridge.is_absolute()));
        // Its own directory is refused as a work folder.
        let own = bridges[0].parent().unwrap();
        if own.components().count() >= 3 {
            assert!(
                check_work_folder(own, &[PathBuf::from("/nonexistent-home")], &bridges).is_err()
            );
        }
    }
}
