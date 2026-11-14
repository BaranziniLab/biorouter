//! The engine against the vendored fixtures, offline (PR-09).
//!
//! * the voice detector finds speech within the 11 s of `jfk.wav`, and none in 5 s of
//!   generated silence or seeded pink noise;
//! * a decode with the zero tensor `for-tests` model returns 0 with empty text;
//! * a set abort flag reads as cancelled even when whisper.cpp returns 0;
//! * a corrupt model header is refused quickly, before any large allocation (the
//!   CVE-2026-110810 class).
//!
//! `jfk_latency_real_models` (ignored) measures load and decode times with real models
//! from `BIOROUTER_DICTATION_BENCH_DIR`.
#![cfg(not(br_no_native))]

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicI32;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use biorouter_dictation::engine::{self, Backend, Engine, LoadOptions};

/// Each load moves the process's current directory; one engine at a time.
fn engine_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("vendor")
        .join("whisper.cpp")
        .join("source")
        .join(relative)
}

fn read_wav16(path: &Path) -> Vec<f32> {
    let bytes = std::fs::read(path).expect("read the WAV");
    let mut at = 12usize;
    while at + 8 <= bytes.len() {
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        if &bytes[at..at + 4] == b"data" {
            return bytes[at + 8..at + 8 + len]
                .chunks_exact(2)
                .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32_768.0)
                .collect();
        }
        at += 8 + len + (len & 1);
    }
    panic!("{} has no data chunk", path.display());
}

fn load(model: &Path, vad: &Path, use_gpu: bool, n_threads: u32) -> Engine {
    let size = std::fs::metadata(model).expect("the model").len();
    Engine::load(&LoadOptions {
        model_path: model,
        model_size: size,
        vad_path: vad,
        use_gpu,
        n_threads,
    })
    .unwrap_or_else(|e| panic!("{} did not load: {e}", model.display()))
}

/// Pink noise from white noise through Paul Kellet's filter, seeded so every run hears
/// the same five seconds, at about -26 dBFS.
fn pink_noise(seconds: usize, mut seed: u64) -> Vec<f32> {
    let mut b = [0f32; 7];
    (0..seconds * 16_000)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let white = (seed >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0;
            b[0] = 0.99886 * b[0] + white * 0.0555179;
            b[1] = 0.99332 * b[1] + white * 0.0750759;
            b[2] = 0.96900 * b[2] + white * 0.153852;
            b[3] = 0.86650 * b[3] + white * 0.3104856;
            b[4] = 0.55000 * b[4] + white * 0.5329522;
            b[5] = -0.7616 * b[5] - white * 0.0168980;
            let pink = b.iter().sum::<f32>() + white * 0.5362;
            b[6] = white * 0.115926;
            pink * 0.05
        })
        .collect()
}

#[test]
fn the_detector_finds_speech_in_jfk_and_none_in_silence_or_noise() {
    let _lock = engine_lock();
    let mut engine = load(
        &fixture("models/for-tests-ggml-base.en.bin"),
        &fixture("models/for-tests-silero-v6.2.0-ggml.bin"),
        false,
        4,
    );
    let params = engine::vad_params();

    let jfk = read_wav16(&fixture("samples/jfk.wav"));
    let spans = engine.vad_spans(&jfk, &params).expect("detector");
    assert!(
        spans.iter().any(|s| s.t0_ms < s.t1_ms && s.t1_ms <= 11_100),
        "no speech span within jfk.wav: {spans:?}"
    );

    let silence = vec![0.0f32; 5 * 16_000];
    assert_eq!(engine.vad_spans(&silence, &params).expect("detector"), []);

    let noise = pink_noise(5, 0x0005_eed0_fd1c_7a7e);
    let rms = (noise.iter().map(|x| x * x).sum::<f32>() / noise.len() as f32).sqrt();
    assert!(rms > 0.01 && rms < 0.2, "rms {rms}");
    assert_eq!(engine.vad_spans(&noise, &params).expect("detector"), []);
}

#[test]
fn the_fixture_model_decodes_jfk_to_empty_text() {
    let _lock = engine_lock();
    let mut engine = load(
        &fixture("models/for-tests-ggml-base.en.bin"),
        &fixture("models/for-tests-silero-v6.2.0-ggml.bin"),
        false,
        4,
    );
    let jfk = read_wav16(&fixture("samples/jfk.wav"));
    let abort = AtomicI32::new(0);
    let output = engine
        .transcribe(
            &jfk,
            &engine::decode_params("en", 4),
            &engine::vad_params(),
            &abort,
        )
        .expect("whisper_full returns 0");
    assert_eq!(output.text, "");

    // More than 25 s is refused before whisper.cpp sees it.
    let long = vec![0.0f32; engine::MAX_TRANSCRIBE_SAMPLES + 1];
    assert!(matches!(
        engine.transcribe(
            &long,
            &engine::decode_params("en", 4),
            &engine::vad_params(),
            &abort
        ),
        Err(engine::EngineError::TooLong)
    ));
}

/// A cancel that lands before the first encoder pass is a cancel, never an empty
/// transcription. whisper.cpp answers a stop at the encoder begin callback with 0 and no
/// segments, so the shim must report the flag itself. The real detector finds speech in
/// `jfk.wav`, so the decode reaches that callback; silence returns before it, and a set
/// flag still wins.
#[test]
fn a_cancel_before_the_first_encoder_pass_is_reported_as_cancelled() {
    let _lock = engine_lock();
    let mut engine = load(
        &fixture("models/for-tests-ggml-base.en.bin"),
        &fixture("models/for-tests-silero-v6.2.0-ggml.bin"),
        false,
        4,
    );
    let jfk = read_wav16(&fixture("samples/jfk.wav"));
    let silence = vec![0.0f32; 2 * 16_000];
    let params = engine::decode_params("en", 4);
    let vad = engine::vad_params();
    let cancelled = AtomicI32::new(1);
    for (name, pcm) in [("jfk.wav", &jfk), ("silence", &silence)] {
        let result = engine.transcribe(pcm, &params, &vad, &cancelled);
        assert!(
            matches!(result, Err(engine::EngineError::Aborted)),
            "{name}: {result:?}"
        );
    }
    // The same engine decodes again once the flag is clear.
    let clear = AtomicI32::new(0);
    let output = engine
        .transcribe(&jfk, &params, &vad, &clear)
        .expect("a decode after a cancel");
    assert_eq!(output.text, "");
}

/// Peak resident size of this process, in bytes.
#[cfg(unix)]
fn peak_rss() -> u64 {
    // SAFETY: getrusage writes into the struct it is given.
    let usage = unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut usage);
        usage
    };
    let max = usage.ru_maxrss.max(0) as u64;
    if cfg!(target_os = "macos") {
        max
    } else {
        max * 1024
    }
}

#[test]
fn a_corrupt_header_is_refused_quickly_without_a_large_allocation() {
    let _lock = engine_lock();
    let dir = tempfile::tempdir().expect("a temporary folder");

    // A real header with a vocabulary of two billion tokens and layers a million wide.
    let mut huge = Vec::new();
    for value in [
        0x6767_6d6c_i32,
        i32::MAX,
        1500,
        1 << 20,
        8,
        1 << 16,
        448,
        1 << 20,
        8,
        1 << 16,
        80,
        1,
    ] {
        huge.extend_from_slice(&value.to_le_bytes());
    }
    huge.resize(4096, 0);

    // Whisper's own base.en header with a tensor record whose name claims 2 GiB.
    let mut long_name =
        std::fs::read(fixture("models/for-tests-ggml-base.en.bin")).expect("fixture");
    for value in [2i32, i32::MAX, 1, 512, 512] {
        long_name.extend_from_slice(&value.to_le_bytes());
    }
    long_name.extend_from_slice(b"encoder.conv1.weight");

    for (name, bytes) in [("huge.bin", huge), ("long-name.bin", long_name)] {
        let path = dir.path().join(name);
        std::fs::write(&path, &bytes).expect("write the corrupt model");
        #[cfg(unix)]
        let before = peak_rss();
        let started = Instant::now();
        let result = Engine::load(&LoadOptions {
            model_path: &path,
            model_size: bytes.len() as u64,
            vad_path: &fixture("models/for-tests-silero-v6.2.0-ggml.bin"),
            use_gpu: false,
            n_threads: 2,
        });
        let elapsed = started.elapsed();
        assert!(
            matches!(result, Err(engine::EngineError::ModelHeader(_))),
            "{name}: {:?}",
            result.err()
        );
        assert!(
            elapsed.as_secs_f64() < 1.0,
            "{name}: refused after {elapsed:?}"
        );
        #[cfg(unix)]
        {
            let grown = peak_rss().saturating_sub(before);
            assert!(grown < 64 << 20, "{name}: peak RSS grew by {grown} bytes");
        }
    }
}

/// The separate detector's time over `jfk.wav` repeated to 11, 60 and 300 s (M1-L4), on
/// its one thread. The vendored detector is the production one, so this needs no
/// download. Run with `--ignored --nocapture`; the load average is printed beside it,
/// because the figures move with it.
#[test]
#[ignore = "a timing; run with --ignored --nocapture"]
fn detector_spans_latency() {
    let _lock = engine_lock();
    let mut engine = load(
        &fixture("models/for-tests-ggml-base.en.bin"),
        &fixture("models/for-tests-silero-v6.2.0-ggml.bin"),
        false,
        engine::default_threads(),
    );
    let params = engine::vad_params();
    let jfk = read_wav16(&fixture("samples/jfk.wav"));
    #[cfg(unix)]
    {
        let mut load = [0f64; 3];
        // SAFETY: getloadavg writes at most three values into the array.
        let n = unsafe { libc::getloadavg(load.as_mut_ptr(), 3) };
        println!("load average {:?}", &load[..n.max(0) as usize]);
    }
    for seconds in [11usize, 60, 300] {
        let pcm: Vec<f32> = jfk.iter().copied().cycle().take(seconds * 16_000).collect();
        let mut times: Vec<f64> = (0..5)
            .map(|_| {
                let t = Instant::now();
                let spans = engine.vad_spans(&pcm, &params).expect("detector");
                assert!(!spans.is_empty());
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        times.sort_by(|a, b| a.total_cmp(b));
        println!(
            "vad_spans {seconds:>3} s: median {:.1} ms, min {:.1} ms, max {:.1} ms",
            times[2], times[0], times[4]
        );
    }
}

/// Load and decode times with the real models on this machine. Run with
/// `BIOROUTER_DICTATION_BENCH_DIR=<folder with models/>` and `--ignored --nocapture`.
/// Every figure is the engine alone: no process start, no pipe.
///
/// `BIOROUTER_DICTATION_BENCH_MODELS` (comma separated file names, the vendored fixture
/// being `for-tests-ggml-base.en.bin`) keeps only those models, so each can be timed in a
/// fresh process, where its first load also pays for the Metal device and its library.
/// `BIOROUTER_DICTATION_BENCH_CLIPS`
/// (paths joined like `PATH`) adds 16 kHz mono 16 bit clips, each decoded twice on the
/// warm engine with its time and text printed.
#[test]
#[ignore = "needs real speech models; set BIOROUTER_DICTATION_BENCH_DIR"]
fn jfk_latency_real_models() {
    let _lock = engine_lock();
    // Absolute before the first load moves the current directory; the engine refuses a
    // relative path.
    let bench = std::path::absolute(PathBuf::from(
        std::env::var_os("BIOROUTER_DICTATION_BENCH_DIR")
            .expect("BIOROUTER_DICTATION_BENCH_DIR names the bench folder"),
    ))
    .expect("an absolute bench folder");
    let vad = bench.join("models").join("ggml-silero-v6.2.0.bin");
    let jfk = read_wav16(&fixture("samples/jfk.wav"));
    let audio_s = jfk.len() as f64 / 16_000.0;
    let runs: usize = std::env::var("BIOROUTER_DICTATION_BENCH_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let only: Option<Vec<String>> = std::env::var("BIOROUTER_DICTATION_BENCH_MODELS")
        .ok()
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect());
    let clips: Vec<(String, Vec<f32>)> = std::env::var_os("BIOROUTER_DICTATION_BENCH_CLIPS")
        .map(|v| {
            std::env::split_paths(&v)
                .map(|p| {
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    (
                        name,
                        read_wav16(&std::path::absolute(&p).expect("a clip path")),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let mut models = Vec::new();
    if only
        .as_ref()
        .is_none_or(|names| names.iter().any(|n| n == "for-tests-ggml-base.en.bin"))
    {
        models.push((
            "for-tests-ggml-base.en".to_string(),
            fixture("models/for-tests-ggml-base.en.bin"),
            fixture("models/for-tests-silero-v6.2.0-ggml.bin"),
        ));
    }
    for name in [
        "ggml-base.en.bin",
        "ggml-small.en.bin",
        "ggml-large-v3-turbo-q5_0.bin",
    ] {
        let path = bench.join("models").join(name);
        let wanted = only
            .as_ref()
            .is_none_or(|names| names.iter().any(|n| n == name));
        if wanted && path.is_file() {
            models.push((name.trim_end_matches(".bin").to_string(), path, vad.clone()));
        }
    }
    let threads = engine::default_threads();
    let mut configs = vec![("cpu", false, threads)];
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        configs.insert(0, ("gpu", true, threads));
    }

    println!(
        "jfk.wav {audio_s:.2} s; {} decodes after the first; threads {threads}",
        runs
    );
    println!(
        "{:<28} {:<5} {:>8} {:>8} {:>8} {:>9} {:>9} {:>9} {:>8} {:>7}  text",
        "model", "cfg", "load_ms", "warm_ms", "first", "median", "min", "max", "enc_ms", "rtf"
    );
    for (name, model, detector) in &models {
        for (label, use_gpu, n_threads) in &configs {
            let started = Instant::now();
            let mut engine = load(model, detector, *use_gpu, *n_threads);
            let load_ms = started.elapsed().as_secs_f64() * 1000.0;
            let report = engine.load_report();
            if *use_gpu {
                assert_eq!(engine.backend(), Backend::Metal);
            }
            let params = engine::decode_params("en", *n_threads);
            let vad_params = engine::vad_params();
            let abort = AtomicI32::new(0);
            let mut times = Vec::new();
            let mut text = String::new();
            let mut encode_ms = 0.0f32;
            for _ in 0..=runs {
                let t = Instant::now();
                let out = engine
                    .transcribe(&jfk, &params, &vad_params, &abort)
                    .expect("decode");
                times.push(t.elapsed().as_secs_f64() * 1000.0);
                text = out.text;
                encode_ms = out.timings.encode_ms;
            }
            let first = times.remove(0);
            times.sort_by(|a, b| a.total_cmp(b));
            let median = times.get(times.len() / 2).copied().unwrap_or(first);
            let min = times.first().copied().unwrap_or(first);
            let max = times.last().copied().unwrap_or(first);
            println!(
                "{:<28} {:<5} {:>8.0} {:>8.0} {:>8.0} {:>9.1} {:>9.1} {:>9.1} {:>8.1} {:>7.3}  {}",
                name,
                label,
                load_ms,
                report.warmup_ms,
                first,
                median,
                min,
                max,
                encode_ms,
                median / 1000.0 / audio_s,
                text
            );
            for (clip, pcm) in &clips {
                let mut row = Vec::new();
                let mut clip_text = String::new();
                let mut split = engine::Timings::default();
                for _ in 0..2 {
                    let t = Instant::now();
                    let out = engine
                        .transcribe(pcm, &params, &vad_params, &abort)
                        .expect("decode");
                    row.push(t.elapsed().as_secs_f64() * 1000.0);
                    clip_text = out.text;
                    split = out.timings;
                }
                println!(
                    "  {:<26} {:<5} {:>5.2} s  {:>7.1} {:>7.1} ms  vad {:>6.1} enc {:>6.1} dec {:>6.1}  {}",
                    clip,
                    label,
                    pcm.len() as f64 / 16_000.0,
                    row[0],
                    row[1],
                    split.vad_ms,
                    split.encode_ms,
                    split.decode_ms,
                    clip_text
                );
            }
        }
    }
}
