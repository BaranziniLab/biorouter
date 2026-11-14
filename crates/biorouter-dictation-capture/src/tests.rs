use super::*;

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use biorouter_dictation::protocol::{read_frame_blocking, FrameError};

/// A microphone that sends `chunk` samples of `value` every 10 ms once started.
struct Fake {
    chunk: usize,
    value: f32,
    fail: Option<ErrorCode>,
    lose_after: Option<Duration>,
    starts: Arc<AtomicUsize>,
    queued_samples: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
    started_at: Option<Instant>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Fake {
    fn new() -> Fake {
        Fake {
            chunk: 800,
            value: 0.25,
            fail: None,
            lose_after: None,
            starts: Arc::new(AtomicUsize::new(0)),
            queued_samples: Arc::new(AtomicUsize::new(0)),
            stopped: Arc::new(AtomicBool::new(false)),
            started_at: None,
            thread: None,
        }
    }
}

impl Backend for Fake {
    fn probe(&mut self) -> Probe {
        Probe {
            compiled: vec!["Fake".into()],
            backend: Some("Fake".into()),
            devices: vec![Device {
                name: "Fake microphone".into(),
                default: true,
            }],
            error: None,
            test_source: false,
        }
    }

    fn start(
        &mut self,
        device: Option<&str>,
        sink: mpsc::Sender<Event>,
    ) -> Result<Ready, ErrorCode> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        if let Some(code) = self.fail {
            return Err(code);
        }
        let (chunk, value, stopped, queued_samples) = (
            self.chunk,
            self.value,
            Arc::clone(&self.stopped),
            Arc::clone(&self.queued_samples),
        );
        self.started_at = Some(Instant::now());
        self.thread = Some(std::thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                if sink.send(Event::Samples(vec![value; chunk])).is_err() {
                    return;
                }
                queued_samples.fetch_add(chunk, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(10));
            }
        }));
        Ok(Ready {
            backend: "Fake".into(),
            device: device.unwrap_or("Fake microphone").into(),
            rate: SAMPLE_RATE,
            device_rate: 48_000,
        })
    }

    fn lost(&self) -> bool {
        match (self.lose_after, self.started_at) {
            (Some(after), Some(at)) => at.elapsed() >= after,
            _ => false,
        }
    }

    fn stop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn frame(message: &ToCapture) -> Vec<u8> {
    let json = to_json(message);
    let mut bytes = Vec::new();
    write_frame_blocking(&mut bytes, &json, &[]).unwrap();
    bytes
}

/// Every frame the child wrote, decoded.
fn read_all(out: &[u8]) -> Vec<(FromCapture, Vec<u8>)> {
    let mut cursor = Cursor::new(out);
    let mut frames = Vec::new();
    loop {
        match read_frame_blocking(&mut cursor) {
            Ok(f) => frames.push((serde_json::from_slice(&f.json).unwrap(), f.payload)),
            Err(FrameError::Closed) => return frames,
            Err(e) => panic!("a frame did not read: {e}"),
        }
    }
}

/// Runs the child over a real pipe: `script` writes to the child's stdin and closes it.
fn run_child(
    backend: &mut dyn Backend,
    agent_shell: bool,
    script: impl FnOnce(&mut io::PipeWriter) + Send + 'static,
) -> (i32, Vec<(FromCapture, Vec<u8>)>, Duration) {
    let (reader, mut writer) = io::pipe().unwrap();
    let feeder = std::thread::spawn(move || {
        script(&mut writer);
        Instant::now()
    });
    let mut out = Vec::new();
    let code = serve(backend, reader, &mut out, agent_shell);
    let returned = Instant::now();
    let closed = feeder.join().unwrap();
    (
        code,
        read_all(&out),
        returned.saturating_duration_since(closed),
    )
}

#[test]
fn messages_have_their_wire_shape() {
    assert_eq!(
        serde_json::to_value(ToCapture::Start { device: None }).unwrap(),
        serde_json::json!({"type": "start", "device": null})
    );
    assert_eq!(
        serde_json::from_str::<ToCapture>(r#"{"type":"start"}"#).unwrap(),
        ToCapture::Start { device: None }
    );
    assert_eq!(
        serde_json::from_str::<ToCapture>(r#"{"type":"later_thing"}"#).unwrap(),
        ToCapture::Unknown
    );
    assert_eq!(
        serde_json::to_value(FromCapture::Error {
            code: ErrorCode::DeviceBusy
        })
        .unwrap(),
        serde_json::json!({"type": "error", "code": "device_busy"})
    );
    assert_eq!(
        serde_json::to_value(FromCapture::Stopped { frames: 3 }).unwrap(),
        serde_json::json!({"type": "stopped", "frames": 3})
    );
    assert_eq!(
        serde_json::to_value(FromCapture::Level(Level {
            peak: 0.5,
            rms: 0.25
        }))
        .unwrap(),
        serde_json::json!({"type": "level", "peak": 0.5, "rms": 0.25})
    );
    let ready = FromCapture::Ready(Ready {
        backend: "Core Audio".into(),
        device: "MacBook Pro Microphone".into(),
        rate: 16_000,
        device_rate: 48_000,
    });
    let json = serde_json::to_string(&ready).unwrap();
    assert_eq!(serde_json::from_str::<FromCapture>(&json).unwrap(), ready);
    assert_eq!(
        serde_json::from_str::<FromCapture>(r#"{"type":"error","code":"from_the_future"}"#)
            .unwrap(),
        FromCapture::Error {
            code: ErrorCode::Unknown
        }
    );
}

#[test]
fn audio_payloads_round_trip_as_f32_le() {
    let samples = [0.0, -1.0, 0.5, f32::MIN_POSITIVE, 0.123_456_79];
    let payload = payload_of(&samples);
    assert_eq!(payload.len(), samples.len() * 4);
    assert_eq!(&payload[4..8], &(-1.0f32).to_le_bytes());
    assert_eq!(samples_of(&payload), samples);
    // A short tail is no sample.
    assert_eq!(samples_of(&payload[..6]).len(), 1);
}

/// P-1: `wipe` zeroes every sample of any buffer the child and the parent drop.
#[test]
fn wipe_zeroes_every_sample() {
    let mut floats = vec![0.25f32, -1.0, f32::MIN_POSITIVE];
    wipe(&mut floats);
    assert_eq!(floats, [0.0; 3]);
    let mut pcm = vec![i16::MAX, -7, 1];
    wipe(&mut pcm);
    assert_eq!(pcm, [0; 3]);
    let mut bytes = payload_of(&[0.5, 0.75]);
    wipe(&mut bytes);
    assert!(bytes.iter().all(|b| *b == 0));
    wipe::<u8>(&mut []);
}

#[test]
fn the_level_is_peak_and_rms() {
    let level = Level::of(&[0.5, -0.5, 0.5, -0.5]);
    assert_eq!(level.peak, 0.5);
    assert!((level.rms - 0.5).abs() < 1e-6);
    assert_eq!(Level::of(&[]), Level::default());
    assert_eq!(Level::of(&[0.0; 160]).peak, 0.0);
}

/// Probe first; then `ready`, a level before every audio frame, the samples in order, and
/// on `stop` everything the device delivered before `stopped`, whose count matches.
#[test]
fn frames_round_trip_from_probe_to_stopped() {
    let mut fake = Fake::new();
    let queued_samples = Arc::clone(&fake.queued_samples);
    let ready_to_stop = Arc::new(AtomicBool::new(false));
    let ready_to_stop_in_script = Arc::clone(&ready_to_stop);
    let (code, frames, _) = run_child(&mut fake, false, move |w| {
        w.write_all(&frame(&ToCapture::Start { device: None }))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while queued_samples.load(Ordering::SeqCst) < 16_000 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        ready_to_stop_in_script.store(
            queued_samples.load(Ordering::SeqCst) >= 16_000,
            Ordering::SeqCst,
        );
        w.write_all(&frame(&ToCapture::Stop)).unwrap();
    });
    assert!(
        ready_to_stop.load(Ordering::SeqCst),
        "fake did not queue 16,000 samples before stop"
    );
    assert_eq!(code, exit::OK);
    assert!(matches!(&frames[0].0, FromCapture::Probe(p) if p.backend.as_deref() == Some("Fake")));
    assert!(matches!(&frames[1].0, FromCapture::Ready(r) if r.rate == 16_000));
    let (last, _) = frames.last().unwrap();
    let FromCapture::Stopped { frames: counted } = last else {
        panic!("the last frame is {last:?}");
    };
    let mut delivered = 0u64;
    let mut expect_audio = false;
    for (message, payload) in &frames[2..frames.len() - 1] {
        match message {
            FromCapture::Level(level) => {
                assert!(!expect_audio, "two levels in a row");
                assert_eq!(level.peak, 0.25);
                expect_audio = true;
            }
            FromCapture::Audio => {
                assert!(expect_audio, "audio without its level");
                let samples = samples_of(payload);
                assert!(samples.iter().all(|s| *s == 0.25));
                delivered += samples.len() as u64;
                expect_audio = false;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(delivered, *counted);
    assert!(*counted >= 16_000, "{counted}");
    assert!(fake.stopped.load(Ordering::SeqCst), "the device was closed");
}

/// How long a test waits for the child to return before it counts as a hang. It bounds a
/// hang and is not a speed claim: a busy CI runner stalls a thread for seconds, so a test
/// asserts why the child ended (its exit code, its last frame), not how fast.
const HANG_GUARD: Duration = Duration::from_secs(30);

/// [`run_child`] with the child on a thread of its own, waited for at most
/// [`HANG_GUARD`], for a test of how a child ends: a child that never returned would hang
/// [`run_child`] for good. The backend comes back for the test's own checks.
fn run_child_within_hang_guard<B: Backend + Send + 'static>(
    mut backend: B,
    script: impl FnOnce(&mut io::PipeWriter) + Send + 'static,
) -> (i32, Vec<(FromCapture, Vec<u8>)>, B) {
    let (reader, mut writer) = io::pipe().unwrap();
    let feeder = std::thread::spawn(move || script(&mut writer));
    let child = std::thread::spawn(move || {
        let mut out = Vec::new();
        let code = serve(&mut backend, reader, &mut out, false);
        (code, out, backend)
    });
    let deadline = Instant::now() + HANG_GUARD;
    while !child.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the child did not return within {HANG_GUARD:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    feeder.join().unwrap();
    let (code, out, backend) = child.join().unwrap();
    (code, read_all(&out), backend)
}

/// Stdin's end of file stops a recording stream as `stop` does, and is what stops it: the
/// device is still delivering when stdin closes, no `stop` is sent, the last frame is
/// `stopped` with the count of every sample that went out, the device is closed, and the
/// child returns 0. There is no wall clock bound: one held the return to 2 s from the
/// close, which on a busy runner measures the load rather than the end of file path. The
/// wait is held to [`HANG_GUARD`].
#[test]
fn end_of_file_stops_a_recording_stream_cleanly() {
    let fake = Fake::new();
    let (queued, device_closed) = (Arc::clone(&fake.queued_samples), Arc::clone(&fake.stopped));
    let (code, frames, fake) = run_child_within_hang_guard(fake, move |w| {
        w.write_all(&frame(&ToCapture::Start { device: None }))
            .unwrap();
        let deadline = Instant::now() + HANG_GUARD;
        while queued.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "the device never delivered");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            !device_closed.load(Ordering::SeqCst),
            "the device closed before stdin did"
        );
        // Returning drops the writer: end of file, and no `stop`.
    });
    assert_eq!(code, exit::OK);
    assert!(matches!(&frames[1].0, FromCapture::Ready(_)), "{frames:?}");
    let (last, _) = frames.last().unwrap();
    let FromCapture::Stopped { frames: counted } = last else {
        panic!("the last frame is {last:?}");
    };
    let sent: u64 = frames
        .iter()
        .filter(|(message, _)| matches!(message, FromCapture::Audio))
        .map(|(_, payload)| samples_of(payload).len() as u64)
        .sum();
    assert_eq!(*counted, sent);
    assert!(sent > 0, "the device delivered before stdin closed");
    assert!(fake.stopped.load(Ordering::SeqCst), "the device was closed");
}

/// Without `start` no device is opened: a probe only, for `devices` and `status`.
#[test]
fn a_probe_alone_opens_nothing() {
    let mut fake = Fake::new();
    let starts = Arc::clone(&fake.starts);
    let (code, frames, _) = run_child(&mut fake, false, |_| {});
    assert_eq!(code, exit::OK);
    assert_eq!(frames.len(), 1);
    assert!(matches!(frames[0].0, FromCapture::Probe(_)));
    assert_eq!(starts.load(Ordering::SeqCst), 0);
}

/// An agent's shell gets no microphone, even by running the hidden role itself.
#[test]
fn the_child_refuses_to_start_in_an_agent_shell() {
    let mut fake = Fake::new();
    let starts = Arc::clone(&fake.starts);
    let (code, frames, _) = run_child(&mut fake, true, |w| {
        w.write_all(&frame(&ToCapture::Start { device: None }))
            .unwrap();
        std::thread::sleep(Duration::from_millis(50));
    });
    assert_eq!(code, exit::REFUSED);
    assert_eq!(starts.load(Ordering::SeqCst), 0);
    assert_eq!(
        frames.last().unwrap().0,
        FromCapture::Error {
            code: ErrorCode::AgentShell
        }
    );
}

#[test]
fn a_device_that_will_not_open_is_one_error_frame() {
    let mut fake = Fake::new();
    fake.fail = Some(ErrorCode::DeviceBusy);
    let (code, frames, _) = run_child(&mut fake, false, |w| {
        w.write_all(&frame(&ToCapture::Start {
            device: Some("USB".into()),
        }))
        .unwrap();
        std::thread::sleep(Duration::from_millis(50));
    });
    assert_eq!(code, exit::FAILED);
    assert_eq!(frames.len(), 2);
    assert_eq!(
        frames[1].0,
        FromCapture::Error {
            code: ErrorCode::DeviceBusy
        }
    );
}

#[test]
fn a_lost_device_is_reported_and_closed() {
    let mut fake = Fake::new();
    fake.lose_after = Some(Duration::from_millis(120));
    let (code, frames, _) = run_child(&mut fake, false, |w| {
        w.write_all(&frame(&ToCapture::Start { device: None }))
            .unwrap();
        std::thread::sleep(Duration::from_millis(600));
    });
    assert_eq!(code, exit::FAILED);
    assert_eq!(
        frames.last().unwrap().0,
        FromCapture::Error {
            code: ErrorCode::DeviceLost
        }
    );
    assert!(fake.stopped.load(Ordering::SeqCst));
}

#[test]
fn an_unreadable_frame_ends_the_child() {
    let mut fake = Fake::new();
    let (code, _, _) = run_child(&mut fake, false, |w| {
        let mut bytes = Vec::new();
        write_frame_blocking(&mut bytes, b"not json", &[]).unwrap();
        w.write_all(&bytes).unwrap();
        std::thread::sleep(Duration::from_millis(50));
    });
    assert_eq!(code, exit::PROTOCOL);
}

/// DD-73: only the backends dictation uses are compiled, and never the Null backend, which
/// records silence where no audio system exists.
#[test]
fn only_the_platforms_backends_are_compiled() {
    let compiled = compiled_backends();
    let want: &[&str] = if cfg!(target_os = "macos") {
        &["Core Audio"]
    } else if cfg!(target_os = "windows") {
        &["WASAPI"]
    } else if cfg!(target_os = "linux") {
        &["PulseAudio", "ALSA"]
    } else {
        &[]
    };
    assert_eq!(compiled, want);
    assert!(!compiled.iter().any(|b| b == "Null"));
}

/// A real context, as `biorouter dictation devices` opens it: listing devices opens no
/// stream, and whatever answers, it is never the Null backend.
#[cfg(not(br_no_capture))]
#[test]
fn a_real_probe_never_answers_with_the_null_backend() {
    let mut backend = Miniaudio::open();
    let probe = backend.probe();
    assert_ne!(probe.backend.as_deref(), Some("Null"), "{probe:?}");
    match &probe.error {
        None => assert!(!probe.devices.is_empty()),
        Some(ErrorCode::NoBackend) => assert!(probe.backend.is_none()),
        Some(ErrorCode::NoDevice) => assert!(probe.devices.is_empty()),
        Some(other) => panic!("a probe answered {other:?}"),
    }
    // ALSA's "null" PCM records zeros, so it is never offered as a device.
    assert!(!probe
        .devices
        .iter()
        .any(|d| d.name.starts_with("Discard all samples")));
}

/// DD-73's escape hatch: the recorder command's stdout is read as s16le, 16 kHz, mono, and
/// its end, unasked, is a lost device.
#[cfg(unix)]
#[test]
fn a_recorder_command_is_read_as_s16le() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("raw.s16");
    let pcm: Vec<i16> = (0..4_001).map(|i: i32| (i * 7 - 9_000) as i16).collect();
    let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
    std::fs::write(&path, &bytes).unwrap();
    let mut recorder = CommandRecorder::new(format!("cat '{}'", path.display()).into());
    let probe = recorder.probe();
    assert_eq!(probe.backend.as_deref(), Some("Recorder"));
    assert_eq!(probe.devices[0].name, "cat");
    let (tx, rx) = mpsc::channel();
    assert_eq!(
        recorder.start(Some("arecord"), tx.clone()).unwrap_err(),
        ErrorCode::DeviceMissing
    );
    let ready = recorder.start(None, tx).unwrap();
    assert_eq!(ready.rate, 16_000);
    let mut got = Vec::new();
    while got.len() < pcm.len() {
        match rx.recv_timeout(HANG_GUARD) {
            Ok(Event::Samples(samples)) => got.extend(samples),
            _ => break,
        }
    }
    let want: Vec<f32> = pcm.iter().map(|&s| f32::from(s) / 32_768.0).collect();
    assert_eq!(got, want);
    let deadline = Instant::now() + HANG_GUARD;
    while !recorder.lost() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(recorder.lost(), "the command ended by itself");
    recorder.stop();
}

/// The test source answers only in a debug build, and stands in for a device by name.
#[test]
fn the_test_source_behaves_like_a_device() {
    let mut source = TestSource::new(OsStr::new("tone"));
    let probe = source.probe();
    assert!(probe.test_source);
    assert_eq!(probe.devices[0].name, TEST_DEVICE);
    let (tx, rx) = mpsc::channel();
    assert_eq!(
        source.start(Some("Another"), tx.clone()).unwrap_err(),
        ErrorCode::DeviceMissing
    );
    let ready = source.start(None, tx).unwrap();
    assert_eq!(ready.device, TEST_DEVICE);
    let Ok(Event::Samples(chunk)) = rx.recv_timeout(Duration::from_secs(2)) else {
        panic!("no samples");
    };
    assert_eq!(chunk.len(), PERIOD_SAMPLES);
    assert!(Level::of(&chunk).peak > 0.2);
    source.stop();
    let zeros = TestSource::new(OsStr::new("zeros"));
    assert!(matches!(zeros.kind, TestKind::Zeros));
    let mut none = TestSource::new(OsStr::new("none"));
    assert_eq!(none.probe().error, Some(ErrorCode::NoBackend));
}

/// P-10: no crate the daemon links depends on capture, and `biorouter-cli` is the only crate
/// that does. Read from every workspace manifest's normal and build dependencies (what
/// `cargo tree -p biorouter-server -e normal,build` walks); `release.sh verify` and
/// `cross-build-nightly` check the built binaries themselves.
#[test]
fn the_daemon_never_depends_on_capture() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut graph: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for entry in std::fs::read_dir(&crates).unwrap() {
        let dir = entry.unwrap().path();
        let manifest = dir.join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        graph.insert(name, linked_path_dependencies(&text));
    }
    assert!(graph.contains_key("biorouter-server"), "{graph:?}");
    let mut reached = std::collections::BTreeSet::new();
    let mut todo = vec!["biorouter-server".to_string()];
    while let Some(next) = todo.pop() {
        if reached.insert(next.clone()) {
            todo.extend(graph.get(&next).cloned().unwrap_or_default());
        }
    }
    assert!(
        !reached.contains("biorouter-dictation-capture"),
        "biorouterd's dependency closure reaches the capture crate: {reached:?}"
    );
    assert!(reached.contains("biorouter-dictation"), "{reached:?}");
    let dependents: Vec<&String> = graph
        .iter()
        .filter(|(_, deps)| deps.iter().any(|d| d == "biorouter-dictation-capture"))
        .map(|(name, _)| name)
        .collect();
    assert_eq!(dependents, ["biorouter-cli"]);
}

use std::path::Path;

/// The sibling crates a manifest names by path outside `[dev-dependencies]`.
fn linked_path_dependencies(manifest: &str) -> Vec<String> {
    let mut linked = Vec::new();
    let mut section = String::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line.trim_matches(|c| c == '[' || c == ']').to_string();
            continue;
        }
        let counts = (section.ends_with("dependencies") || section.contains("dependencies."))
            && !section.contains("dev-dependencies");
        if !counts || line.starts_with('#') {
            continue;
        }
        if let Some((_, rest)) = line.split_once("path = \"../") {
            if let Some((name, _)) = rest.split_once('"') {
                linked.push(name.trim_end_matches('/').to_string());
            }
        }
    }
    linked
}

#[test]
fn the_manifest_reader_skips_dev_dependencies() {
    let manifest = r#"
[dependencies]
a = { path = "../a" }
# b = { path = "../b" }
[target.'cfg(unix)'.dependencies]
c = { path = "../c", optional = true }
[build-dependencies]
d = { path = "../d" }
[dev-dependencies]
e = { path = "../e" }
[dependencies.f]
path = "../f"
"#;
    assert_eq!(linked_path_dependencies(manifest), ["a", "c", "d", "f"]);
}
