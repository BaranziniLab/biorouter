//! The real dictation worker under the real supervisor (PR-10), and `biorouter dictation
//! status`.
//!
//! * a worker killed in the middle of a 600 s `vad_segments` gives `WorkerExited`, and
//!   the next job runs in a new worker;
//! * a model whose header asks for an impossible vocabulary is refused within 2 s, and
//!   the worker that refused it then loads the fixture with a peak resident size within
//!   64 MiB of a worker that loaded only the fixture;
//! * end of file on stdin ends a loaded worker within 1 s;
//! * on Unix the worker leads a process group of its own, so a terminal's Ctrl-C to the
//!   parent's group does not reach it;
//! * a worker run leaves the child's `BIOROUTER_PATH_ROOT` byte identical, a corrupt
//!   `config.yaml` and a legacy folder marker included (the worker reads no
//!   configuration and moves nothing, DD-4);
//! * `biorouter dictation status --format json` is schema 1 and reports
//!   `capture.reason == "not_compiled"`, and `doctor` carries the same object;
//! * under a managed `enabled: false`, `status --verbose` starts no worker and says why;
//! * `worker_latency_real_models` and `cold_transcribe_latency` (ignored) measure the
//!   latency plan's M1-L1 to M1-L8 against real speech models (with warm decodes of
//!   2 s, 5 s and 11 s clips split into detector, encoder and decoder time, the peak
//!   footprint, and where a streamed dictation was cut), and a cold `transcribe_with`
//!   against the fixture;
//! * `a_worker_killed_mid_transcription_real_model` (ignored) kills the worker during a
//!   real Metal transcription (one retry on the CPU) and a CPU one (`WorkerExited`, then
//!   a new worker), and `the_idle_unload_follows_config_yaml` (ignored) checks that
//!   `BIOROUTER_DICTATION_IDLE_UNLOAD_SECS` in `config.yaml` stops the service's worker.
//!
//! The worker's environment (no daemon secret, `GGML_NO_BACKTRACE=1`) is checked where
//! the command is built, in `biorouter::dictation::service`.
#![cfg(feature = "local-dictation")]

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root before
// `main`, as every test binary here does.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use biorouter_dictation::audio::{self, PcmSlice, Span};
use biorouter_dictation::build_info::{fixtures, sha256_file};
use biorouter_dictation::protocol::{
    self, DecodeOptions, ErrorCode, FromWorker, Load, Loaded, ToWorker, Transcribe, VadOptions,
    PROMPT_BUDGET_TOKENS,
};
use biorouter_dictation::supervisor::{
    CommandSpawner, LoadSpec, Supervisor, SupervisorConfig, SupervisorError, TranscribeParams,
};
use biorouter_dictation::worker::WORKER_MARKER;
use biorouter_dictation::ENGINE_COMPILED;

fn biorouter() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_biorouter"))
}

fn fixture(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("vendor")
        .join("whisper.cpp")
        .join("source")
        .join(relative)
}

/// The command the core service builds, aimed at this crate's `biorouter`.
fn spawner() -> Arc<CommandSpawner> {
    Arc::new(CommandSpawner::new(
        biorouter().display().to_string(),
        || biorouter::dictation::service::worker_command(&biorouter()),
    ))
}

fn fixture_spec(n_threads: u32) -> LoadSpec {
    let model = fixture(fixtures::BASE_EN);
    LoadSpec {
        model_size: std::fs::metadata(&model).expect("the fixture model").len(),
        model_path: model,
        model_sha256: fixtures::BASE_EN_SHA256.to_string(),
        vad_path: fixture(fixtures::SILERO),
        use_gpu: false,
        n_threads,
    }
}

fn read_wav_16k(path: &Path) -> Vec<i16> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    audio::parse_wav_s16(&bytes)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .to_16k_mono()
}

fn jfk() -> Arc<[i16]> {
    read_wav_16k(&fixture(fixtures::JFK)).into()
}

/// On macOS the first process this test binary starts takes about 3 s, whatever it runs
/// (measured 2026-11-05: the first worker's `hello` 3.0 s, the next ones 81 to 91 ms;
/// with the corrupt header test alone, a first `--version` absorbed the same 3 s). It
/// is the system's, not the engine's: `biorouter dictation status --verbose`, whose
/// parent is the shipped CLI, starts a worker and reads its `hello` in 0.41 s end to
/// end. The tests that time the worker start one throwaway process first.
fn first_exec_done() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = Command::new(biorouter())
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_killed_worker_gives_a_typed_error_and_the_next_job_a_new_worker() {
    if !ENGINE_COMPILED {
        return;
    }
    let supervisor = Supervisor::start(spawner(), SupervisorConfig::default());
    let spec = Arc::new(fixture_spec(2));
    supervisor
        .prepare(Arc::clone(&spec))
        .await
        .expect("the fixture model loads");
    let first = supervisor.worker_pid().expect("a running worker");

    // 600 s of audio keeps the single threaded detector busy for about a second.
    let long: Arc<[i16]> = vec![0i16; audio::MAX_STREAM_SAMPLES].into();
    let (_, answer) = supervisor.submit_vad_segments(
        Arc::clone(&spec),
        PcmSlice::whole(long),
        VadOptions::dd48(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !supervisor.status().busy {
        assert!(Instant::now() < deadline, "the job was never dispatched");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    supervisor.kill_worker();
    let result = answer.await.expect("answered");
    assert!(
        matches!(result, Err(SupervisorError::WorkerExited { .. })),
        "{result:?}"
    );

    let spans = supervisor
        .vad_segments(
            Arc::clone(&spec),
            PcmSlice::whole(jfk()),
            VadOptions::dd48(),
        )
        .await
        .expect("a new worker answers");
    assert!(!spans.is_empty());
    assert!(spans.iter().all(|s| s.t1_ms <= 11_100));
    let second = supervisor.worker_pid().expect("a running worker");
    assert_ne!(first, second);
}

/// A model whose header asks for an impossible vocabulary and layers a million wide: a
/// loader that trusted it would allocate gigabytes before reading a tensor.
fn corrupt_model(dir: &Path) -> (PathBuf, u64) {
    let model = dir.join("corrupt.bin");
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0x6767_6d6cu32.to_le_bytes());
    for value in [
        i32::MAX,
        1500,
        1 << 20,
        8,
        4096,
        448,
        1 << 20,
        8,
        4096,
        80,
        1,
    ] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.resize(64 * 1024, 0);
    std::fs::write(&model, &bytes).expect("write the model");
    (model, bytes.len() as u64)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_corrupt_model_header_is_refused_within_2_s() {
    if !ENGINE_COMPILED {
        return;
    }
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (model, size) = corrupt_model(dir.path());
    let spec = LoadSpec {
        model_path: model,
        model_sha256: "0".repeat(64),
        model_size: size,
        ..fixture_spec(2)
    };
    first_exec_done();
    let supervisor = Supervisor::start(spawner(), SupervisorConfig::default());
    let started = Instant::now();
    let result = supervisor.prepare(Arc::new(spec)).await;
    let took = started.elapsed();
    let Err(SupervisorError::ModelLoadFailed(reason)) = result else {
        panic!("{result:?}")
    };
    assert!(reason.starts_with("model could not be loaded"), "{reason}");
    assert!(took < Duration::from_secs(2), "{took:?}");
}

/// A frame client over the real worker's pipes.
struct Worker {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    stdout: std::process::ChildStdout,
}

impl Worker {
    fn start(root: Option<&Path>) -> Worker {
        first_exec_done();
        let mut command = Command::new(biorouter());
        command
            .args([WORKER_MARKER, "engine"])
            .env("GGML_NO_BACKTRACE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(root) = root {
            command
                .env("BIOROUTER_PATH_ROOT", root)
                .env("HOME", root.join("home"))
                .current_dir(root);
        }
        let mut child = command.spawn().expect("start the worker");
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("stdout");
        let mut worker = Worker {
            child,
            stdin,
            stdout,
        };
        let hello = worker.recv();
        assert!(matches!(hello, FromWorker::Hello(h) if h.protocol == 1));
        worker
    }

    fn send(&mut self, message: &ToWorker, payload: &[u8]) {
        let stdin = self.stdin.as_mut().expect("stdin is open");
        protocol::write_frame_blocking(stdin, &protocol::to_json(message), payload)
            .expect("write a frame");
    }

    fn recv(&mut self) -> FromWorker {
        let frame = protocol::read_frame_blocking(&mut self.stdout).expect("a frame");
        serde_json::from_slice(&frame.json).expect("a message")
    }

    fn load(&mut self, model: &Path, sha256: &str, size: u64) -> FromWorker {
        self.send(
            &ToWorker::Load(Load {
                model_path: model.to_str().expect("UTF-8").into(),
                model_sha256: sha256.into(),
                model_size: size,
                vad_dir: fixture("models").to_str().expect("UTF-8").into(),
                vad_file: "for-tests-silero-v6.2.0-ggml.bin".into(),
                use_gpu: false,
                n_threads: 2,
            }),
            &[],
        );
        self.recv()
    }

    fn load_fixture(&mut self) -> Loaded {
        let spec = fixture_spec(2);
        match self.load(&spec.model_path, &spec.model_sha256, spec.model_size) {
            FromWorker::Loaded(loaded) => loaded,
            other => panic!("expected loaded, got {other:?}"),
        }
    }

    /// Closes stdin and waits for the exit.
    fn finish(mut self) {
        self.stdin = None;
        assert_eq!(self.exit_within(Duration::from_secs(5)), Some(0));
    }

    /// The exit code, waiting at most `limit`.
    fn exit_within(&mut self, limit: Duration) -> Option<i32> {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                return status.code();
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("the worker did not exit within {limit:?}");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// "Refused before any large allocation", measured: the worker that refused the corrupt
/// header goes on to load the fixture, and its peak resident size since it started
/// (`loaded.rss_bytes`) stays within 64 MiB of a worker that loaded only the fixture,
/// the bound PR-09's offline test holds `Engine::load` to.
#[test]
fn a_refused_header_leaves_no_large_allocation_behind() {
    if !ENGINE_COMPILED {
        return;
    }
    let dir = tempfile::tempdir().expect("a temporary folder");
    let (model, size) = corrupt_model(dir.path());

    let mut plain = Worker::start(None);
    let baseline = plain
        .load_fixture()
        .rss_bytes
        .expect("a peak resident size");
    plain.finish();

    let mut worker = Worker::start(None);
    match worker.load(&model, &"0".repeat(64), size) {
        FromWorker::Error(e) => {
            assert_eq!(e.code, ErrorCode::ModelLoadFailed);
            assert!(e.message.starts_with("model could not be loaded"), "{e:?}");
        }
        other => panic!("the corrupt header was not refused: {other:?}"),
    }
    let after = worker
        .load_fixture()
        .rss_bytes
        .expect("a peak resident size");
    worker.finish();
    assert!(
        after < baseline + (64 << 20),
        "peak resident size {after} B after the refusal, {baseline} B without it"
    );
}

/// The worker leads its own process group, so a terminal's Ctrl-C to the parent's
/// foreground group never reaches it (an interactive `biorouter` catches Ctrl-C to cancel
/// a turn and keeps its warm worker).
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_worker_leads_a_process_group_of_its_own() {
    if !ENGINE_COMPILED {
        return;
    }
    first_exec_done();
    let supervisor = Supervisor::start(spawner(), SupervisorConfig::default());
    supervisor.spawn_ahead();
    wait_for_hello(&supervisor).await;
    let pid = supervisor.worker_pid().expect("a running worker") as libc::pid_t;
    // SAFETY: plain reads of two process groups.
    let (worker_group, own_group) = unsafe { (libc::getpgid(pid), libc::getpgid(0)) };
    assert_eq!(worker_group, pid, "the worker leads its group");
    assert_ne!(worker_group, own_group);
    supervisor.unload().await;
}

#[test]
fn end_of_file_ends_a_loaded_worker_within_1_s() {
    if !ENGINE_COMPILED {
        return;
    }
    let mut worker = Worker::start(None);
    worker.load_fixture();
    let started = Instant::now();
    worker.stdin = None;
    assert_eq!(worker.exit_within(Duration::from_secs(1)), Some(0));
    assert!(started.elapsed() < Duration::from_secs(1));
    let mut rest = Vec::new();
    worker.stdout.read_to_end(&mut rest).expect("read");
    assert!(rest.is_empty(), "nothing follows end of file");
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read the folder") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                files.insert(path.clone(), Vec::new());
                stack.push(path);
            } else {
                files.insert(path.clone(), std::fs::read(&path).expect("read"));
            }
        }
    }
    files
}

#[test]
fn a_worker_run_leaves_configuration_and_the_legacy_marker_alone() {
    if !ENGINE_COMPILED {
        return;
    }
    let root = tempfile::tempdir().expect("a temporary folder");
    let config = root.path().join("config");
    std::fs::create_dir_all(&config).expect("config");
    std::fs::write(config.join("config.yaml"), "::: not yaml\n\t- [\n").expect("config.yaml");
    let data = root.path().join("data");
    std::fs::create_dir_all(&data).expect("data");
    std::fs::write(data.join(".folder-move-pending"), "").expect("marker");
    std::fs::create_dir_all(root.path().join("home")).expect("home");
    let before = snapshot(root.path());

    let mut worker = Worker::start(Some(root.path()));
    worker.load_fixture();
    let jfk = jfk();
    worker.send(
        &ToWorker::Transcribe(Transcribe {
            id: 1,
            bias: None,
            prompt_terms: Vec::new(),
            prompt_budget_tokens: PROMPT_BUDGET_TOKENS,
            language: "en".into(),
            decode: DecodeOptions::dd47(2),
            vad: VadOptions::dd48(),
        }),
        &audio::s16le_bytes(&jfk),
    );
    assert!(matches!(worker.recv(), FromWorker::Result(r) if r.id == 1));
    worker.send(&ToWorker::Shutdown, &[]);
    assert_eq!(worker.exit_within(Duration::from_secs(5)), Some(0));

    assert_eq!(
        snapshot(root.path()),
        before,
        "the worker wrote, moved or created something under its BIOROUTER_PATH_ROOT"
    );
}

fn sandboxed(args: &[&str], root: &Path) -> std::process::Output {
    sandboxed_with_worker(args, root, None)
}

/// `biorouter` under `root`, with `BIOROUTER_DICTATION_WORKER` set to `worker` or unset.
fn sandboxed_with_worker(
    args: &[&str],
    root: &Path,
    worker: Option<&Path>,
) -> std::process::Output {
    std::fs::create_dir_all(root.join("home")).expect("home");
    let mut command = Command::new(biorouter());
    command
        .args(args)
        .env("BIOROUTER_PATH_ROOT", root)
        .env("HOME", root.join("home"))
        .env("BIOROUTER_DISABLE_KEYRING", "true")
        .env_remove("BIOROUTER_DICTATION_WORKER")
        .stdin(Stdio::null());
    if let Some(worker) = worker {
        command.env("BIOROUTER_DICTATION_WORKER", worker);
    }
    command.output().expect("run biorouter")
}

fn assert_schema_1(status: &serde_json::Value) {
    assert_eq!(status["schema"], 1);
    let engine = &status["engine"];
    assert_eq!(engine["compiled"], ENGINE_COMPILED);
    let state = engine["state"].as_str().expect("a state");
    assert!(
        [
            "no_engine",
            "unsupported_cpu",
            "stopped",
            "starting",
            "ready",
            "error"
        ]
        .contains(&state),
        "{state}"
    );
    assert!(engine["threads"]
        .as_u64()
        .is_some_and(|n| (1..=8).contains(&n)));
    assert!(engine["missing_cpu_features"].is_array());
    assert!(engine.get("rss_bytes").is_some());
    if ENGINE_COMPILED {
        assert_eq!(engine["build"]["whisper"], "4da27c41");
        assert_eq!(engine["build"]["ggml"], "0.28.0");
    }
    assert_eq!(status["models"], serde_json::json!([]));
    assert_eq!(status["active_model"], serde_json::Value::Null);
    assert_eq!(status["language"], "en");
    assert_eq!(status["policy"]["managed"], false);
    assert_eq!(
        status["policy"]["enabled"],
        serde_json::json!({"allowed": true, "reason": null})
    );
    assert_eq!(
        status["capture"],
        serde_json::json!({"available": false, "reason": "not_compiled"})
    );
    for key in ["account", "host", "models_dir"] {
        assert!(status[key].as_str().is_some_and(|s| !s.is_empty()), "{key}");
    }
}

#[test]
fn dictation_status_is_schema_1_and_doctor_carries_it() {
    let root = tempfile::tempdir().expect("a temporary folder");
    let output = sandboxed(&["dictation", "status", "--format", "json"], root.path());
    assert!(output.status.success(), "{output:?}");
    let status: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("status prints JSON");
    assert_schema_1(&status);
    if ENGINE_COMPILED {
        // Reading the status starts nothing.
        assert_eq!(status["engine"]["state"], "stopped");
    }
    assert!(status["models_dir"]
        .as_str()
        .is_some_and(|dir| Path::new(dir).starts_with(root.path())));

    let text = sandboxed(&["dictation", "status"], root.path());
    assert!(text.status.success());
    let text = String::from_utf8_lossy(&text.stdout);
    for label in [
        "Dictation",
        "Engine:",
        "Processor:",
        "Worker:",
        "Microphone:",
    ] {
        assert!(text.contains(label), "{label} is missing from {text}");
    }

    let doctor = sandboxed(&["doctor", "--format", "json", "--no-update"], root.path());
    assert!(doctor.status.success(), "{doctor:?}");
    let doctor: serde_json::Value = serde_json::from_slice(&doctor.stdout).expect("doctor JSON");
    let carried = &doctor["dictation"];
    assert_schema_1(carried);
    let keys = |v: &serde_json::Value| {
        v.as_object()
            .map(|o| o.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
    };
    assert_eq!(keys(carried), keys(&status));
    assert_eq!(keys(&carried["engine"]), keys(&status["engine"]));
}

/// Under a managed `enabled: false`, `status --verbose` starts no worker: the worker
/// binary named here does not exist, so a probe that ran would report that it could not
/// start. The same command without the policy file does report that, which proves the
/// check can fail.
#[test]
fn managed_off_status_starts_no_worker() {
    let root = tempfile::tempdir().expect("a temporary folder");
    let managed = root.path().join("managed");
    std::fs::create_dir_all(&managed).expect("the managed folder");
    let policy = managed.join("managed-policy.yaml");
    std::fs::write(&policy, "dictation:\n  enabled: false\n").expect("the policy");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // The trust check refuses a policy others can write.
        for (path, mode) in [(managed.as_path(), 0o755), (policy.as_path(), 0o644)] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
                .expect("set the mode");
        }
    }
    let missing = root.path().join("no-such-worker");
    let args = ["dictation", "status", "--format", "json", "--verbose"];

    let output = sandboxed_with_worker(&args, root.path(), Some(&missing));
    assert!(output.status.success(), "{output:?}");
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
    let off = biorouter::managed::MANAGED_DICTATION_OFF;
    assert_eq!(
        status["policy"]["enabled"],
        serde_json::json!({"allowed": false, "reason": off})
    );
    assert_eq!(status["probe"]["error"], off, "{status}");
    assert_eq!(status["probe"]["hello"], serde_json::Value::Null);
    let text = sandboxed_with_worker(
        &["dictation", "status", "--verbose"],
        root.path(),
        Some(&missing),
    );
    assert!(text.status.success());
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(
        text.lines()
            .any(|line| line.contains("Worker hello:") && line.contains(off)),
        "{text}"
    );

    std::fs::remove_file(&policy).expect("remove the policy");
    let output = sandboxed_with_worker(&args, root.path(), Some(&missing));
    assert!(output.status.success(), "{output:?}");
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
    let error = status["probe"]["error"].as_str().expect("a probe error");
    assert_ne!(error, off);
    if ENGINE_COMPILED {
        assert!(error.contains("could not start"), "{error}");
    }
}

#[test]
fn verbose_status_shows_the_workers_hello() {
    if !ENGINE_COMPILED {
        return;
    }
    let root = tempfile::tempdir().expect("a temporary folder");
    let output = sandboxed(
        &["dictation", "status", "--format", "json", "--verbose"],
        root.path(),
    );
    assert!(output.status.success(), "{output:?}");
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
    let hello = &status["probe"]["hello"];
    assert_eq!(hello["protocol"], 1, "{status}");
    assert_eq!(hello["whisper"], "4da27c41");
    assert_eq!(status["probe"]["exit_code"], 0);
}

// ---------------------------------------------------------------------------------
// M1-L1 to M1-L8: the latency plan of the M1 engine design (section 15.3).
// ---------------------------------------------------------------------------------

fn median_p90_max(mut values: Vec<f64>) -> (f64, f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    values.sort_by(f64::total_cmp);
    let at =
        |q: f64| values[((q * values.len() as f64).ceil() as usize).clamp(1, values.len()) - 1];
    (at(0.5), at(0.9), values[values.len() - 1])
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

/// One model on one backend, measured from a fresh supervisor.
#[derive(serde::Serialize, Default)]
struct Row {
    model: String,
    backend: String,
    threads: u32,
    /// The first worker this test process started: the model file may still be cold.
    first_in_run: bool,
    spawn_to_hello_ms: f64,
    hello_to_loaded_ms: f64,
    load_ms: u64,
    warmup_ms: u64,
    worker_backend: String,
    /// The worker's peak resident size when it answered `loaded`.
    loaded_rss_bytes: Option<u64>,
    /// The worker's peak physical footprint after every request of the row (macOS).
    peak_footprint_bytes: Option<u64>,
    first_request_ms: f64,
    steady_median_ms: f64,
    steady_p90_ms: f64,
    steady_max_ms: f64,
    real_time_factor: f64,
    clips: usize,
    /// Warm decodes of one clip each of about 2 s, 5 s and 11 s: median and fastest.
    by_length: Vec<LengthRow>,
    vad_60s_ms: f64,
    rearm_ms: f64,
    stream_stop_to_text_ms: Option<f64>,
    stream_undecoded_at_stop_s: Option<f64>,
    stream_segments: Option<usize>,
    stream_text_chars: Option<usize>,
    stream_tail_vad_ms: Option<f64>,
    stream_final_segments_s: Option<Vec<f64>>,
    stream_busy_at_stop: Option<bool>,
    stream_final_worker_ms: Option<f64>,
    stream_cuts: Option<Vec<(f64, f64, f64)>>,
    sample_text: String,
}

#[derive(serde::Serialize, Default)]
struct LengthRow {
    clip: String,
    audio_s: f64,
    runs: usize,
    median_ms: f64,
    min_ms: f64,
    /// The worker's own encode plus decode time of the median run.
    worker_ms: f64,
    /// The median run's split: the integrated detector, the encoder, the decoder.
    vad_ms: f64,
    encode_ms: f64,
    decode_ms: f64,
}

/// The worker's peak physical footprint since it started, read from the parent.
#[cfg(target_os = "macos")]
fn peak_footprint_bytes(pid: u32) -> Option<u64> {
    // SAFETY: `rusage_info_v4` is plain data and zero is a valid start; the call fills
    // the struct it is handed and keeps no pointer.
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::proc_pid_rusage(
            pid as libc::c_int,
            libc::RUSAGE_INFO_V4,
            (&mut info as *mut libc::rusage_info_v4).cast::<libc::rusage_info_t>(),
        )
    };
    (rc == 0).then_some(info.ri_lifetime_max_phys_footprint)
}

#[cfg(not(target_os = "macos"))]
fn peak_footprint_bytes(_pid: u32) -> Option<u64> {
    None
}

/// What a streamed replay measured.
struct StreamOutcome {
    stop_ms: f64,
    undecoded_s: f64,
    segments: usize,
    text: String,
    /// The detector's pass over the undecoded tail, at stop.
    tail_vad_ms: f64,
    /// The segments planned at stop, in seconds of audio.
    final_segments_s: Vec<f64>,
    /// A segment cut during the replay was still decoding when stop was pressed.
    busy_at_stop: bool,
    /// The worker's encode plus decode time of the segments planned at stop.
    final_worker_ms: f64,
    /// Each segment cut while the replay played: the audio buffered at the cut, and the
    /// segment, in seconds.
    cuts: Vec<(f64, f64, f64)>,
}

async fn wait_for_hello(supervisor: &Supervisor) {
    let mut status = supervisor.subscribe();
    tokio::time::timeout(
        Duration::from_secs(60),
        status.wait_for(|s| s.hello.is_some()),
    )
    .await
    .expect("hello within 60 s")
    .expect("the supervisor runs");
}

/// M1-L5: the 60 s recording replayed in real time, batched every 2.5 s, with closed
/// segments decoded while it plays and only the tail at stop.
async fn stream_replay(
    supervisor: &Supervisor,
    spec: &Arc<LoadSpec>,
    recording: &Arc<[i16]>,
    threads: u32,
) -> StreamOutcome {
    let whole = PcmSlice::whole(Arc::clone(recording));
    let total_ms = whole.duration_ms();
    let batch_ms = 2_500u64;
    let mut mark = 0u64;
    let mut end = 0u64;
    let mut pending = Vec::new();
    let mut segments = 0usize;
    let mut cuts = Vec::new();
    let started = Instant::now();
    let spans_of = |range: std::ops::Range<u64>| {
        let tail = whole.slice_ms(&range);
        async move {
            supervisor
                .vad_segments(Arc::clone(spec), tail, VadOptions::dd48())
                .await
                .expect("spans")
                .into_iter()
                .map(|s| Span {
                    t0_ms: s.t0_ms + range.start,
                    t1_ms: s.t1_ms + range.start,
                })
                .collect::<Vec<_>>()
        }
    };
    while end < total_ms {
        end = (end + batch_ms).min(total_ms);
        let due = started + Duration::from_millis(end);
        tokio::time::sleep_until(due.into()).await;
        if end == total_ms {
            break;
        }
        if end >= mark + audio::STREAM_VAD_TRIGGER_MS {
            let spans = spans_of(mark..end).await;
            let (closed, next) = audio::plan_closed_segments(&spans, mark, end);
            for range in closed {
                segments += 1;
                cuts.push((
                    end as f64 / 1000.0,
                    range.start as f64 / 1000.0,
                    range.end as f64 / 1000.0,
                ));
                pending.push(
                    supervisor
                        .submit_transcribe(
                            Arc::clone(spec),
                            1,
                            whole.slice_ms(&range),
                            TranscribeParams::new("en", threads),
                        )
                        .1,
                );
            }
            mark = next;
        }
    }
    // Stop: the person pressed stop with `total_ms` buffered.
    let stop = Instant::now();
    let busy_at_stop = supervisor.status().busy;
    let undecoded_s = (total_ms - mark) as f64 / 1000.0;
    let spans = spans_of(mark..total_ms).await;
    let tail_vad_ms = ms(stop);
    let earlier = pending.len();
    let mut final_segments_s = Vec::new();
    for range in audio::plan_final_segments(&spans, mark, total_ms) {
        segments += 1;
        final_segments_s.push((range.end - range.start) as f64 / 1000.0);
        pending.push(
            supervisor
                .submit_transcribe(
                    Arc::clone(spec),
                    1,
                    whole.slice_ms(&range),
                    TranscribeParams::new("en", threads),
                )
                .1,
        );
    }
    let mut text = Vec::new();
    let mut final_worker_ms = 0.0;
    for (i, answer) in pending.into_iter().enumerate() {
        let result = answer.await.expect("answered").expect("a transcript");
        if i >= earlier {
            final_worker_ms += f64::from(result.timings.encode_ms + result.timings.decode_ms);
        }
        text.push(result.text);
    }
    StreamOutcome {
        stop_ms: ms(stop),
        undecoded_s,
        segments,
        text: text.join(" "),
        tail_vad_ms,
        final_segments_s,
        busy_at_stop,
        final_worker_ms,
        cuts,
    }
}

fn self_test_elapsed(binary: &Path, cpu: bool) -> Option<u64> {
    let mut command = Command::new(binary);
    command
        .args([WORKER_MARKER, "self-test", "--model"])
        .arg(fixture(fixtures::BASE_EN))
        .arg("--vad")
        .arg(fixture(fixtures::SILERO))
        .arg("--wav")
        .arg(fixture(fixtures::JFK));
    if cpu {
        command.arg("--cpu");
    }
    let output = command.output().ok()?;
    let line: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    line["elapsed_ms"].as_u64()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs real speech models; set BIOROUTER_DICTATION_BENCH_DIR"]
async fn worker_latency_real_models() {
    let bench = PathBuf::from(
        std::env::var_os("BIOROUTER_DICTATION_BENCH_DIR")
            .expect("BIOROUTER_DICTATION_BENCH_DIR names the bench folder"),
    );
    let models = bench.join("models");
    let vad = models.join("ggml-silero-v6.2.0.bin");
    let stream = std::env::var("BIOROUTER_DICTATION_LATENCY_STREAM").as_deref() == Ok("1");
    let clip_limit: usize = std::env::var("BIOROUTER_DICTATION_LATENCY_CLIPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX);
    // `turbo q5_0/gpu,small.en f16/cpu8` runs only those configurations.
    let only: Option<Vec<String>> = std::env::var("BIOROUTER_DICTATION_LATENCY_ONLY")
        .ok()
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect());
    let wanted = |label: &str, backend: &str| {
        only.as_ref()
            .is_none_or(|only| only.iter().any(|o| *o == format!("{label}/{backend}")))
    };

    // The first exec of a freshly built binary pays for the page cache: discard it.
    let _ = Command::new(biorouter()).arg("--version").output();

    let mut clips: Vec<(String, Arc<[i16]>)> = Vec::new();
    for n in 1..=12 {
        for voice in ["Daniel", "Karen", "Samantha"] {
            let name = format!("s{n:02}_{voice}");
            let pcm = read_wav_16k(&bench.join("audio").join(format!("{name}.wav")));
            clips.push((name, pcm.into()));
        }
    }
    clips.truncate(clip_limit.max(1));
    let first_clip = read_wav_16k(&bench.join("audio").join("s01_Daniel.wav"));
    let mut sixty: Vec<i16> = Vec::new();
    for n in 1..=12 {
        sixty.extend(read_wav_16k(
            &bench.join("audio").join(format!("s{n:02}_Daniel.wav")),
        ));
        sixty.extend(std::iter::repeat_n(0i16, 11_200));
    }
    let sixty: Arc<[i16]> = sixty.into();
    // One clip of each length the targets name: the first 2 s of a 6 s sentence, a 5 s
    // sentence, and the 11 s jfk.wav.
    let six = read_wav_16k(&bench.join("audio").join("s02_Daniel.wav"));
    let lengths: Vec<(&str, Arc<[i16]>)> = vec![
        ("2 s", six.get(..32_000).unwrap_or(&six).to_vec().into()),
        (
            "5 s",
            read_wav_16k(&bench.join("audio").join("s11_Daniel.wav")).into(),
        ),
        ("11 s", jfk()),
    ];
    let mut first_in_run = true;

    let gpu_host = cfg!(all(target_os = "macos", target_arch = "aarch64"));
    let mut backends: Vec<(&str, bool, u32)> = Vec::new();
    if gpu_host {
        backends.push(("gpu", true, 8));
    }
    backends.push(("cpu8", false, 8));
    backends.push(("cpu4", false, 4));

    let mut rows = Vec::new();
    for (file, label) in [
        ("ggml-large-v3-turbo-q5_0.bin", "turbo q5_0"),
        ("ggml-small.en.bin", "small.en f16"),
        ("ggml-base.en.bin", "base.en f16"),
    ] {
        let path = models.join(file);
        let Ok(meta) = std::fs::metadata(&path) else {
            eprintln!("skipping {file}: not in {}", models.display());
            continue;
        };
        if !backends
            .iter()
            .any(|(backend, _, _)| wanted(label, backend))
        {
            continue;
        }
        let sha = sha256_file(&path).expect("hash the model");
        let language = "en";
        for (backend, use_gpu, threads) in &backends {
            if !wanted(label, backend) {
                continue;
            }
            if label.starts_with("turbo")
                && *backend == "cpu4"
                && std::env::var("BIOROUTER_DICTATION_LATENCY_TURBO_CPU4").as_deref() != Ok("1")
            {
                eprintln!("skipping turbo on cpu4 (set BIOROUTER_DICTATION_LATENCY_TURBO_CPU4=1)");
                continue;
            }
            let spec = Arc::new(LoadSpec {
                model_path: path.clone(),
                model_sha256: sha.clone(),
                model_size: meta.len(),
                vad_path: vad.clone(),
                use_gpu: *use_gpu,
                n_threads: *threads,
            });
            let supervisor = Supervisor::start(spawner(), SupervisorConfig::default());
            let mut row = Row {
                model: label.into(),
                backend: (*backend).into(),
                threads: *threads,
                first_in_run: std::mem::take(&mut first_in_run),
                ..Row::default()
            };

            // M1-L1: spawn to hello, hello to loaded.
            let started = Instant::now();
            supervisor.spawn_ahead();
            wait_for_hello(&supervisor).await;
            row.spawn_to_hello_ms = ms(started);
            let hello_at = Instant::now();
            let loaded = supervisor
                .prepare(Arc::clone(&spec))
                .await
                .expect("the model loads");
            row.hello_to_loaded_ms = ms(hello_at);
            row.load_ms = loaded.load_ms;
            row.warmup_ms = loaded.warmup_ms;
            row.worker_backend = loaded.backend.clone();
            row.loaded_rss_bytes = loaded.rss_bytes;

            // M1-L2: the first request after loaded.
            let at = Instant::now();
            let first = supervisor
                .transcribe(
                    Arc::clone(&spec),
                    1,
                    PcmSlice::whole(first_clip.clone().into()),
                    TranscribeParams::new(language, *threads),
                )
                .await
                .expect("the first request");
            row.first_request_ms = ms(at);
            row.sample_text = first.text.clone();

            // M1-L3: steady state.
            let mut walls = Vec::new();
            let mut audio_s = 0.0;
            let mut wall_s = 0.0;
            for (name, pcm) in &clips {
                let at = Instant::now();
                let result = supervisor
                    .transcribe(
                        Arc::clone(&spec),
                        1,
                        PcmSlice::whole(Arc::clone(pcm)),
                        TranscribeParams::new(language, *threads),
                    )
                    .await
                    .expect("a transcript");
                let wall = ms(at);
                assert!(!result.text.is_empty(), "{name} decoded to nothing");
                walls.push(wall);
                wall_s += wall / 1000.0;
                audio_s += pcm.len() as f64 / 16_000.0;
            }
            let (median, p90, max) = median_p90_max(walls);
            row.steady_median_ms = median;
            row.steady_p90_ms = p90;
            row.steady_max_ms = max;
            row.real_time_factor = wall_s / audio_s.max(f64::EPSILON);
            row.clips = clips.len();

            // Warm decodes by length.
            let runs = if *use_gpu { 5 } else { 2 };
            for (clip, pcm) in &lengths {
                let mut walls = Vec::new();
                let mut workers = Vec::new();
                for _ in 0..runs {
                    let at = Instant::now();
                    let result = supervisor
                        .transcribe(
                            Arc::clone(&spec),
                            1,
                            PcmSlice::whole(Arc::clone(pcm)),
                            TranscribeParams::new(language, *threads),
                        )
                        .await
                        .expect("a transcript");
                    walls.push(ms(at));
                    workers.push(result.timings);
                }
                let mut order: Vec<usize> = (0..walls.len()).collect();
                order.sort_by(|a, b| walls[*a].total_cmp(&walls[*b]));
                let mid = order[(order.len() - 1) / 2];
                let split = &workers[mid];
                row.by_length.push(LengthRow {
                    clip: (*clip).into(),
                    audio_s: pcm.len() as f64 / 16_000.0,
                    runs,
                    median_ms: walls[mid],
                    min_ms: walls[order[0]],
                    worker_ms: f64::from(split.encode_ms + split.decode_ms),
                    vad_ms: f64::from(split.vad_ms),
                    encode_ms: f64::from(split.encode_ms),
                    decode_ms: f64::from(split.decode_ms),
                });
            }

            // M1-L4: the detector on 60 s.
            let at = Instant::now();
            let spans = supervisor
                .vad_segments(
                    Arc::clone(&spec),
                    PcmSlice::whole(Arc::clone(&sixty)),
                    VadOptions::dd48(),
                )
                .await
                .expect("spans");
            row.vad_60s_ms = ms(at);
            assert!(spans.len() >= 12, "{} spans in 60 s", spans.len());

            // M1-L5: streamed stop to text, on the two configurations the plan names.
            let streamed = stream
                && ((label.starts_with("turbo") && *backend == "gpu")
                    || (label.starts_with("small") && *backend == "cpu8"));
            if streamed {
                let outcome = stream_replay(&supervisor, &spec, &sixty, *threads).await;
                row.stream_stop_to_text_ms = Some(outcome.stop_ms);
                row.stream_undecoded_at_stop_s = Some(outcome.undecoded_s);
                row.stream_segments = Some(outcome.segments);
                row.stream_text_chars = Some(outcome.text.len());
                row.stream_tail_vad_ms = Some(outcome.tail_vad_ms);
                row.stream_final_segments_s = Some(outcome.final_segments_s);
                row.stream_busy_at_stop = Some(outcome.busy_at_stop);
                row.stream_final_worker_ms = Some(outcome.final_worker_ms);
                row.stream_cuts = Some(outcome.cuts);
            }
            row.peak_footprint_bytes = supervisor.worker_pid().and_then(peak_footprint_bytes);

            // M1-L6: re-arm after an unload, with a warm file cache.
            supervisor.unload().await;
            let at = Instant::now();
            supervisor
                .prepare(Arc::clone(&spec))
                .await
                .expect("the model loads again");
            row.rearm_ms = ms(at);
            supervisor.unload().await;

            println!(
                "{:<13} {:<5} t={} hello {:>6.1} ms | loaded {:>7.1} ms (load {} + warmup {} ms, {}) | first {:>7.1} ms | steady median {:>7.1} p90 {:>7.1} max {:>7.1} ms, rtf {:.3} over {} clips | vad 60 s {:>6.1} ms | re-arm {:>7.1} ms{}",
                row.model,
                row.backend,
                row.threads,
                row.spawn_to_hello_ms,
                row.hello_to_loaded_ms,
                row.load_ms,
                row.warmup_ms,
                row.worker_backend,
                row.first_request_ms,
                row.steady_median_ms,
                row.steady_p90_ms,
                row.steady_max_ms,
                row.real_time_factor,
                row.clips,
                row.vad_60s_ms,
                row.rearm_ms,
                match (row.stream_stop_to_text_ms, row.stream_undecoded_at_stop_s, row.stream_segments) {
                    (Some(stop), Some(undecoded), Some(segments)) => format!(
                        " | stream stop to text {stop:.1} ms, {undecoded:.1} s undecoded at stop, {segments} segments"
                    ),
                    _ => String::new(),
                }
            );
            println!("    first text: {}", row.sample_text);
            println!(
                "    {}spawn to ready {:.1} ms | peak rss at loaded {} MB, peak footprint {} MB",
                if row.first_in_run {
                    "first worker of the run: "
                } else {
                    ""
                },
                row.spawn_to_hello_ms + row.hello_to_loaded_ms,
                row.loaded_rss_bytes.map_or(0, |b| b / 1_000_000),
                row.peak_footprint_bytes.map_or(0, |b| b / 1_000_000),
            );
            for length in &row.by_length {
                println!(
                    "    warm {:>4} ({:.2} s): median {:>7.1} ms, fastest {:>7.1} ms over {} runs; worker: detector {:.1} + encoder {:.1} + decoder {:.1} ms",
                    length.clip,
                    length.audio_s,
                    length.median_ms,
                    length.min_ms,
                    length.runs,
                    length.vad_ms,
                    length.encode_ms,
                    length.decode_ms,
                );
            }
            if let (Some(vad), Some(finals), Some(busy), Some(worker)) = (
                row.stream_tail_vad_ms,
                row.stream_final_segments_s.as_ref(),
                row.stream_busy_at_stop,
                row.stream_final_worker_ms,
            ) {
                println!(
                    "    stream at stop: tail detection {vad:.1} ms, final segments {finals:?} s, worker {worker:.1} ms, a cut segment still decoding: {busy}"
                );
            }
            if let Some(cuts) = &row.stream_cuts {
                let shown: Vec<String> = cuts
                    .iter()
                    .map(|(at, from, to)| format!("@{at:.1}: {from:.2}-{to:.2}"))
                    .collect();
                println!(
                    "    stream cuts (buffered: segment, s): {}",
                    shown.join(", ")
                );
            }
            rows.push(row);
        }
    }

    // M1-L7: the warm up's effect is the first request against the steady median.
    for row in &rows {
        println!(
            "M1-L7 {:<13} {:<5} first {:.1} ms vs steady median {:.1} ms ({:+.1} ms)",
            row.model,
            row.backend,
            row.first_request_ms,
            row.steady_median_ms,
            row.first_request_ms - row.steady_median_ms
        );
    }

    // M1-L8: the self test, both binaries, CPU and the default backend.
    let daemon = biorouter().with_file_name(if cfg!(windows) {
        "biorouterd.exe"
    } else {
        "biorouterd"
    });
    let mut self_tests = Vec::new();
    for binary in [biorouter(), daemon] {
        if !binary.exists() {
            continue;
        }
        for cpu in [true, false] {
            let elapsed = self_test_elapsed(&binary, cpu);
            println!(
                "M1-L8 {} self test{}: {:?} ms",
                binary.file_name().unwrap_or_default().to_string_lossy(),
                if cpu { " --cpu" } else { "" },
                elapsed
            );
            self_tests.push(serde_json::json!({
                "binary": binary.file_name().unwrap_or_default().to_string_lossy(),
                "cpu": cpu,
                "elapsed_ms": elapsed,
            }));
        }
    }

    if let Some(out) = std::env::var_os("BIOROUTER_DICTATION_LATENCY_OUT") {
        let json = serde_json::json!({"rows": rows, "self_tests": self_tests});
        std::fs::write(&out, serde_json::to_vec_pretty(&json).expect("JSON"))
            .expect("write the latency file");
    }
    assert!(!rows.is_empty(), "no model was measured");
}

/// Waits until a job is running, then kills the worker as a crash would. Returns the
/// killed worker's pid and when the kill was sent.
async fn kill_once_busy(supervisor: &Supervisor) -> (u32, Instant) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !supervisor.status().busy {
        assert!(Instant::now() < deadline, "the job was never dispatched");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let pid = supervisor.worker_pid().expect("a running worker");
    supervisor.kill_worker();
    (pid, Instant::now())
}

/// A worker killed in the middle of a real transcription. During a Metal job DD-7
/// retries that job once on the CPU in a fresh worker, which serves until the next idle
/// unload; during a CPU job the job fails with `WorkerExited` and the next request gets a
/// fresh worker. An unload then clears the move to the CPU.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs real speech models; set BIOROUTER_DICTATION_BENCH_DIR"]
async fn a_worker_killed_mid_transcription_real_model() {
    let bench = PathBuf::from(
        std::env::var_os("BIOROUTER_DICTATION_BENCH_DIR")
            .expect("BIOROUTER_DICTATION_BENCH_DIR names the bench folder"),
    );
    let models = bench.join("models");
    let path = models.join("ggml-large-v3-turbo-q5_0.bin");
    let size = std::fs::metadata(&path).expect("the turbo model").len();
    let gpu = cfg!(all(target_os = "macos", target_arch = "aarch64"));
    let spec = Arc::new(LoadSpec {
        model_sha256: sha256_file(&path).expect("hash the model"),
        model_path: path,
        model_size: size,
        vad_path: models.join("ggml-silero-v6.2.0.bin"),
        use_gpu: gpu,
        n_threads: 8,
    });
    first_exec_done();
    let supervisor = Supervisor::start(spawner(), SupervisorConfig::default());
    let loaded = supervisor
        .prepare(Arc::clone(&spec))
        .await
        .expect("the model loads");
    let pcm = jfk();
    let request = || {
        supervisor.submit_transcribe(
            Arc::clone(&spec),
            1,
            PcmSlice::whole(Arc::clone(&pcm)),
            TranscribeParams::new("en", 8),
        )
    };

    if gpu {
        assert_eq!(loaded.backend, "metal");
        let (_, answer) = request();
        let (killed, at) = kill_once_busy(&supervisor).await;
        let result = answer.await.expect("answered");
        let retried = result.expect("the Metal job is retried once on the CPU");
        assert!(retried.text.contains("country"), "{}", retried.text);
        let status = supervisor.status();
        assert!(status.force_cpu, "{status:?}");
        assert_eq!(status.backend.as_deref(), Some("cpu"));
        assert_ne!(status.pid, Some(killed));
        println!(
            "killed a Metal job: retried on the CPU in a new worker, answered {:.1} ms after the kill",
            ms(at)
        );
    }

    let (_, answer) = request();
    let (killed, at) = kill_once_busy(&supervisor).await;
    let result = answer.await.expect("answered");
    let took = ms(at);
    assert!(
        matches!(result, Err(SupervisorError::WorkerExited { code: None })),
        "{result:?}"
    );
    println!(
        "killed a CPU job: {} ({took:.1} ms after the kill)",
        result.expect_err("an error")
    );
    let at = Instant::now();
    let next = request().1.await.expect("answered").expect("a new worker");
    assert!(next.text.contains("country"), "{}", next.text);
    let fresh = supervisor.worker_pid().expect("a running worker");
    assert_ne!(fresh, killed);
    println!(
        "the next request ran in a new worker (pid {killed} then {fresh}) in {:.1} ms",
        ms(at)
    );

    if gpu {
        supervisor.unload().await;
        let again = supervisor
            .prepare(Arc::clone(&spec))
            .await
            .expect("the model loads again");
        assert_eq!(
            again.backend, "metal",
            "the unload clears the move to the CPU"
        );
        assert!(!supervisor.status().force_cpu);
    }
    supervisor.unload().await;
}

/// `BIOROUTER_DICTATION_IDLE_UNLOAD_SECS` in `config.yaml` (DD-91) reaches the process's
/// own service: a worker loaded through `prepare_with` is stopped, and its process gone,
/// once that many idle seconds pass. Ignored because it owns the process's one service
/// and its configuration file for several seconds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "owns the process's dictation service; run with --ignored"]
async fn the_idle_unload_follows_config_yaml() {
    use biorouter::dictation::EngineState;
    if !ENGINE_COMPILED {
        return;
    }
    let config = PathBuf::from(biorouter::config::Config::global().path());
    assert!(
        !config.exists(),
        "the sandbox already has {}",
        config.display()
    );
    std::fs::create_dir_all(config.parent().expect("a folder")).expect("the config folder");
    std::fs::write(&config, "BIOROUTER_DICTATION_IDLE_UNLOAD_SECS: 2\n").expect("config.yaml");
    std::env::set_var(biorouter::dictation::service::WORKER_ENV, biorouter());
    first_exec_done();

    let service = biorouter::dictation::service();
    service
        .prepare_with(fixture_spec(2))
        .await
        .expect("the fixture model loads");
    let status = service.status();
    assert_eq!(status.state, EngineState::Ready, "{status:?}");
    let loaded_at = Instant::now();
    let mut stopped = None;
    while loaded_at.elapsed() < Duration::from_secs(10) {
        if service.status().state == EngineState::Stopped {
            stopped = Some(loaded_at.elapsed());
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    std::fs::remove_file(&config).expect("remove config.yaml");
    let stopped = stopped.expect("the worker is stopped after the configured idle time");
    assert!(
        stopped >= Duration::from_millis(1_900) && stopped < Duration::from_secs(4),
        "stopped after {stopped:?}"
    );
    println!("the idle unload stopped the worker {stopped:?} after it was ready");
}

/// A cold `transcribe_with` (nothing loaded, no `prepare_with` before it) through the
/// process's own service, against the fixture model, two ways: the thread count worked
/// out first and the request sent after it, as the service did before it overlapped
/// them, and `n_threads: 0`, where the service starts the worker before it asks the
/// sidecar. Each round waits out the 10 s reuse of the thread count, so both pay for
/// the sidecar probe.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "a timing; run with --ignored --nocapture"]
async fn cold_transcribe_latency() {
    use biorouter::dictation::TranscribeRequest;
    if !ENGINE_COMPILED {
        return;
    }
    std::env::set_var(biorouter::dictation::service::WORKER_ENV, biorouter());
    first_exec_done();
    let service = biorouter::dictation::service();
    let pcm = jfk();
    let rounds: usize = std::env::var("BIOROUTER_DICTATION_LATENCY_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    let (mut probes, mut serial, mut overlapped) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..rounds {
        tokio::time::sleep(Duration::from_millis(10_200)).await;
        let at = Instant::now();
        let threads = service.threads().await;
        probes.push(ms(at));
        service
            .transcribe_with(
                fixture_spec(threads),
                1,
                PcmSlice::whole(Arc::clone(&pcm)),
                TranscribeRequest::new("en"),
            )
            .await
            .expect("a transcript");
        serial.push(ms(at));
        service.unload().await;

        tokio::time::sleep(Duration::from_millis(10_200)).await;
        let at = Instant::now();
        service
            .transcribe_with(
                fixture_spec(0),
                1,
                PcmSlice::whole(Arc::clone(&pcm)),
                TranscribeRequest::new("en"),
            )
            .await
            .expect("a transcript");
        overlapped.push(ms(at));
        service.unload().await;
    }
    let show = |name: &str, values: Vec<f64>| {
        let (median, p90, max) = median_p90_max(values);
        println!("{name:<38} median {median:>7.1} ms, p90 {p90:>7.1} ms, max {max:>7.1} ms");
    };
    show("sidecar probe (DD-32 thread count)", probes);
    show("cold transcribe, probe then request", serial);
    show("cold transcribe, worker started first", overlapped);
}
