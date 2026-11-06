//! The offline self test every package and release smoke runs (DD-12):
//!
//! ```text
//! <binary> __br-dictation-worker self-test --model M --vad V --wav W [--cpu]
//! ```
//!
//! It proves, with no network and no real model, that this binary's engine starts on this
//! processor, carries the instruction set it was built for, loads a model through the
//! engine's own loader, finds speech with the detector, and decodes. On an old processor
//! it exits 20 with MSG-CPU instead of crashing. It prints exactly one JSON line on
//! stdout:
//!
//! ```json
//! {"selftest":"ok","whisper":"4da27c41","ggml":"0.28.0","isa":"x86-64-v3","backend":"cpu","elapsed_ms":130}
//! ```

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicI32;
use std::time::Instant;

use serde::Serialize;

use super::exit;
use crate::build_info::{self, fixtures};
use crate::catalog::{Catalog, SttKind, CATALOG};
use crate::cpu::{self, CpuReport, HardwareClass};
use crate::engine::{self, Backend, Engine, LoadOptions};

/// jfk.wav is 11.0 s long; a span "within the clip" ends by then.
const JFK_END_MS: u64 = 11_100;

#[derive(Serialize)]
struct Passed<'a> {
    selftest: &'static str,
    whisper: &'static str,
    ggml: &'static str,
    isa: &'static str,
    backend: &'a str,
    elapsed_ms: u64,
}

#[derive(Serialize)]
struct Failed {
    selftest: &'static str,
    stage: &'static str,
    exit: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    missing: Option<Vec<&'static str>>,
}

struct Args {
    model: PathBuf,
    vad: PathBuf,
    wav: PathBuf,
    cpu: bool,
}

/// The role's entry point: this process's CPU, stdout and stderr.
pub fn run(args: &[OsString]) -> i32 {
    let mut out = private_stdout();
    let mut err = std::io::stderr();
    run_with(args, &mut *out, &mut err, cpu::host_report())
}

/// The self test with its outputs and the CPU gate's answer passed in, for the tests.
pub fn run_with(
    args: &[OsString],
    out: &mut dyn Write,
    err: &mut dyn Write,
    cpu_report: &CpuReport,
) -> i32 {
    let started = Instant::now();

    let args = match parse(args) {
        Ok(args) => args,
        Err(why) => {
            let _ = writeln!(
                err,
                "{why}\nusage: {} self-test --model FILE --vad FILE --wav FILE [--cpu]",
                super::WORKER_MARKER
            );
            return fail(out, "args", exit::NO_SUCH_ROLE, None);
        }
    };

    // 1. The CPU gate, before anything calls into ggml.
    if cpu_report.class == HardwareClass::No {
        if let Some(sentence) = cpu::sentence(cpu_report) {
            let _ = writeln!(err, "{sentence}");
        }
        return fail(
            out,
            "cpu",
            exit::UNSUPPORTED_CPU,
            Some(cpu_report.missing.clone()),
        );
    }

    // 2. Only pinned files (P-11): a release binary carries no "load any file" path.
    let model = absolute(&args.model);
    let vad = absolute(&args.vad);
    let wav = absolute(&args.wav);
    if !is_pinned(&model, &fixtures::MODEL_PINS, SttKind::Speech, &CATALOG)
        || !is_pinned(&vad, &fixtures::DETECTOR_PINS, SttKind::Vad, &CATALOG)
    {
        let _ = writeln!(
            err,
            "The self test accepts only pinned files: the fixtures and the speech model catalog. The model or the voice detector file is missing, unreadable or not pinned."
        );
        return fail(out, "pin", exit::NOT_PINNED, None);
    }
    let pcm = match read_wav(&wav) {
        Ok(pcm) => pcm,
        Err(why) => {
            let _ = writeln!(err, "The sample could not be read: {why}.");
            return fail(out, "args", exit::NO_SUCH_ROLE, None);
        }
    };

    // 3. The instruction set this build was configured with is really compiled in.
    engine::install_logging();
    let missing = isa_missing(&engine::system_info());
    if !missing.is_empty() {
        let _ = writeln!(
            err,
            "This build lacks instruction set features it was configured with: {}.",
            missing.join(", ")
        );
        return fail(out, "isa", exit::CHECK_FAILED, None);
    }

    // 4. Load through the engine's own loader.
    let threads = engine::default_threads();
    let use_gpu = !args.cpu && cfg!(all(target_os = "macos", target_arch = "aarch64"));
    let model_size = std::fs::metadata(&model).map(|m| m.len()).unwrap_or(0);
    let mut engine = match Engine::load(&LoadOptions {
        model_path: &model,
        model_size,
        vad_path: &vad,
        use_gpu,
        n_threads: threads,
    }) {
        Ok(engine) => engine,
        Err(e) => {
            let _ = writeln!(err, "The self test could not load its model: {e}.");
            return fail(out, "load", exit::CHECK_FAILED, None);
        }
    };

    // 5. The separate detector: speech in the sample, none in silence.
    let vad_params = engine::vad_params();
    let speech = engine.vad_spans(&pcm, &vad_params);
    let silence = engine.vad_spans(&vec![0.0f32; 5 * 16_000], &vad_params);
    let found_speech = speech.as_ref().is_ok_and(|spans| {
        spans
            .iter()
            .any(|s| s.t0_ms < s.t1_ms && s.t1_ms <= JFK_END_MS)
    });
    let quiet_silence = silence.as_ref().is_ok_and(|spans| spans.is_empty());
    if !found_speech || !quiet_silence {
        let _ = writeln!(
            err,
            "The voice detector did not find speech in the sample, or found speech in silence."
        );
        return fail(out, "vad", exit::CHECK_FAILED, None);
    }

    // 6. A decode with the production parameters, integrated detector on. The fixture
    // model has no tensors, so the text is empty [M].
    let abort = AtomicI32::new(0);
    let decoded = engine.transcribe(
        &pcm,
        &engine::decode_params("en", threads),
        &vad_params,
        &abort,
    );
    match &decoded {
        Ok(output) if output.text.is_empty() => {}
        Ok(_) => {
            let _ = writeln!(
                err,
                "Decoding the sample with the fixture model returned text."
            );
            return fail(out, "decode", exit::CHECK_FAILED, None);
        }
        Err(e) => {
            let _ = writeln!(err, "Decoding the sample failed: {e}.");
            return fail(out, "decode", exit::CHECK_FAILED, None);
        }
    }

    let backend = engine.backend();
    drop(engine);
    let line = Passed {
        selftest: "ok",
        whisper: build_info::WHISPER_COMMIT,
        ggml: build_info::GGML_VERSION,
        isa: build_info::ISA,
        backend: backend_name(backend),
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    write_line(out, &line);
    exit::OK
}

fn backend_name(backend: Backend) -> &'static str {
    backend.as_str()
}

fn fail(
    out: &mut dyn Write,
    stage: &'static str,
    code: i32,
    missing: Option<Vec<&'static str>>,
) -> i32 {
    write_line(
        out,
        &Failed {
            selftest: "failed",
            stage,
            exit: code,
            missing,
        },
    );
    code
}

fn write_line<T: Serialize>(out: &mut dyn Write, value: &T) {
    if let Ok(mut line) = serde_json::to_string(value) {
        line.push('\n');
        let _ = out.write_all(line.as_bytes());
        let _ = out.flush();
    }
}

fn parse(args: &[OsString]) -> Result<Args, String> {
    let mut model = None;
    let mut vad = None;
    let mut wav = None;
    let mut cpu = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let slot = match arg.to_str() {
            Some("--model") => &mut model,
            Some("--vad") => &mut vad,
            Some("--wav") => &mut wav,
            Some("--cpu") => {
                cpu = true;
                continue;
            }
            _ => return Err(format!("unknown argument {}", arg.to_string_lossy())),
        };
        let value = it
            .next()
            .ok_or_else(|| format!("{} needs a file", arg.to_string_lossy()))?;
        *slot = Some(PathBuf::from(value));
    }
    Ok(Args {
        model: model.ok_or("--model is required")?,
        vad: vad.ok_or("--vad is required")?,
        wav: wav.ok_or("--wav is required")?,
        cpu,
    })
}

/// Loading the engine moves the current directory to the detector's folder, so every
/// path is made absolute first.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Every pinned fixture is under 1 MiB, so a file up to this size is hashed against the
/// fixture pins.
const MAX_FIXTURE_BYTES: u64 = 4 << 20;

/// A fixture (`pins`), or a catalog file of `kind` by its pin or an accepted earlier
/// revision (the M1 design's step 2, extended by PR-11). A file is hashed only when its
/// size is a fixture's or at most the largest catalog file of `kind`; anything larger
/// is refused for the cost of a `stat`.
fn is_pinned(path: &Path, pins: &[&str], kind: SttKind, catalog: &Catalog) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let of_kind = || catalog.entries.iter().filter(move |e| e.kind == kind);
    let ceiling = of_kind()
        .map(|e| e.size_bytes)
        .max()
        .unwrap_or(0)
        .max(MAX_FIXTURE_BYTES);
    if !meta.is_file() || meta.len() > ceiling {
        return false;
    }
    build_info::sha256_file(path).is_ok_and(|digest| {
        pins.contains(&digest.as_str()) || of_kind().any(|e| e.accepts(&digest))
    })
}

/// The features the ggml CPU backend reports that this build's ISA promises but lacks.
/// ggml reports compile time features (`ggml_cpu_has_avx2` tests `__AVX2__`), so this
/// catches a build that silently fell back to SSE2 or to the NEON baseline.
fn isa_missing(system_info: &str) -> Vec<&'static str> {
    let required: &[&'static str] = if cfg!(target_arch = "x86_64") {
        &["AVX2", "FMA", "F16C", "BMI2"]
    } else if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
        &["DOTPROD", "FP16_VA"]
    } else if cfg!(target_arch = "aarch64") {
        &["DOTPROD"]
    } else {
        &[]
    };
    let present: Vec<&str> = system_info.split('|').map(str::trim).collect();
    required
        .iter()
        .copied()
        .filter(|feature| {
            let wanted = format!("{feature} = 1");
            !present.iter().any(|item| {
                item.ends_with(&wanted) && {
                    // "CPU : NEON = 1" carries the backend's name before the first feature.
                    let head = item.len() - wanted.len();
                    head == 0 || item.get(..head).is_some_and(|h| h.ends_with(": "))
                }
            })
        })
        .collect()
}

/// A RIFF file whose format chunk says 16 kHz, mono, 16 bit PCM; its `data` chunk as
/// samples in [-1, 1).
fn read_wav(path: &Path) -> Result<Vec<f32>, &'static str> {
    let bytes = std::fs::read(path).map_err(|_| "the file is not readable")?;
    if bytes.get(..4) != Some(b"RIFF".as_slice()) || bytes.get(8..12) != Some(b"WAVE".as_slice()) {
        return Err("it is not a WAV file");
    }
    let u16_at = |at: usize| -> Option<u16> {
        Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
    };
    let u32_at = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    let mut format_ok = None;
    let mut at = 12usize;
    while let (Some(id), Some(len)) = (bytes.get(at..at + 4), u32_at(at + 4)) {
        let body = at + 8;
        let len = len as usize;
        match id {
            b"fmt " => {
                let ok = u16_at(body) == Some(1)
                    && u16_at(body + 2) == Some(1)
                    && u32_at(body + 4) == Some(16_000)
                    && u16_at(body + 14) == Some(16);
                format_ok = Some(ok);
            }
            b"data" => {
                if format_ok != Some(true) {
                    return Err("it is not 16 kHz mono 16 bit PCM");
                }
                let data = bytes
                    .get(body..body + len)
                    .ok_or("its data chunk is cut short")?;
                return Ok(data
                    .chunks_exact(2)
                    .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32_768.0)
                    .collect());
            }
            _ => {}
        }
        at = body + len + (len & 1);
    }
    Err("it has no data chunk")
}

/// stdout for the one result line. On Unix the process's own stdout is pointed at stderr
/// first, so a stray `printf` from C or C++ code can never add a line the smokes would
/// read.
#[cfg(unix)]
fn private_stdout() -> Box<dyn Write> {
    use std::os::fd::FromRawFd;
    // SAFETY: plain descriptor calls; the duplicate is owned by the returned File.
    unsafe {
        let fd = libc::dup(1);
        if fd < 0 {
            return Box::new(std::io::stdout());
        }
        if libc::dup2(2, 1) < 0 {
            libc::close(fd);
            return Box::new(std::io::stdout());
        }
        Box::new(std::fs::File::from_raw_fd(fd))
    }
}

#[cfg(not(unix))]
fn private_stdout() -> Box<dyn Write> {
    Box::new(std::io::stdout())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::{classify_for, CpuFeatures, Platform};
    use crate::engine::test_support::engine_lock;

    fn fixture_args(model: &Path) -> Vec<OsString> {
        vec![
            "--model".into(),
            model.into(),
            "--vad".into(),
            fixtures::path(fixtures::SILERO).into(),
            "--wav".into(),
            fixtures::path(fixtures::JFK).into(),
            "--cpu".into(),
        ]
    }

    fn run_capturing(args: &[OsString], report: &CpuReport) -> (i32, String, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(args, &mut out, &mut err, report);
        (
            code,
            String::from_utf8(out).expect("stdout is UTF-8"),
            String::from_utf8(err).expect("stderr is UTF-8"),
        )
    }

    #[test]
    fn the_self_test_prints_one_line_and_passes() {
        let _lock = engine_lock();
        let (code, out, err) = run_capturing(
            &fixture_args(&fixtures::path(fixtures::BASE_EN)),
            cpu::host_report(),
        );
        assert_eq!(code, exit::OK, "stdout {out:?} stderr {err:?}");
        assert_eq!(out.lines().count(), 1, "{out:?}");
        let line: serde_json::Value = serde_json::from_str(out.trim()).expect("one JSON line");
        let keys: Vec<&str> = line
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        let mut sorted_keys = keys.clone();
        sorted_keys.sort_unstable();
        assert_eq!(
            sorted_keys,
            [
                "backend",
                "elapsed_ms",
                "ggml",
                "isa",
                "selftest",
                "whisper"
            ]
        );
        assert!(
            out.starts_with(r#"{"selftest":"ok","whisper":"4da27c41","ggml":"0.28.0","isa":""#),
            "{out}"
        );
        assert_eq!(line["backend"], "cpu");
        assert_eq!(line["isa"], build_info::ISA);
    }

    #[test]
    fn the_tiny_fixture_passes_too() {
        let _lock = engine_lock();
        let (code, out, err) = run_capturing(
            &fixture_args(&fixtures::path(fixtures::TINY)),
            cpu::host_report(),
        );
        assert_eq!(code, exit::OK, "stdout {out:?} stderr {err:?}");
    }

    #[test]
    fn an_unsupported_processor_exits_20_with_the_sentence() {
        let refused = classify_for(Platform::X86, &CpuFeatures::default(), false, None);
        let (code, out, err) =
            run_capturing(&fixture_args(&fixtures::path(fixtures::BASE_EN)), &refused);
        assert_eq!(code, exit::UNSUPPORTED_CPU);
        assert!(
            err.starts_with(
                "This processor does not support the instructions local dictation needs ("
            ),
            "{err}"
        );
        assert_eq!(
            out,
            "{\"selftest\":\"failed\",\"stage\":\"cpu\",\"exit\":20,\"missing\":[\"avx2\",\"fma\",\"f16c\",\"bmi2\"]}\n"
        );
    }

    #[test]
    fn a_model_with_one_byte_flipped_is_not_pinned() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let copy = dir.path().join("for-tests-ggml-base.en.bin");
        let mut bytes = std::fs::read(fixtures::path(fixtures::BASE_EN)).expect("fixture");
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        std::fs::write(&copy, bytes).expect("write the copy");
        let (code, out, _) = run_capturing(&fixture_args(&copy), cpu::host_report());
        assert_eq!(code, exit::NOT_PINNED);
        assert_eq!(
            out,
            "{\"selftest\":\"failed\",\"stage\":\"pin\",\"exit\":21}\n"
        );
    }

    #[test]
    fn a_file_larger_than_any_fixture_is_not_pinned() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let big = dir.path().join("ggml-large.bin");
        let file = std::fs::File::create(&big).expect("create");
        file.set_len(MAX_FIXTURE_BYTES + 1).expect("a sparse file");
        drop(file);
        let (code, out, _) = run_capturing(&fixture_args(&big), cpu::host_report());
        assert_eq!(code, exit::NOT_PINNED);
        assert!(out.contains("\"stage\":\"pin\""), "{out}");
        assert!(
            [fixtures::BASE_EN, fixtures::TINY, fixtures::SILERO]
                .iter()
                .all(|f| std::fs::metadata(fixtures::path(f))
                    .is_ok_and(|m| m.len() <= MAX_FIXTURE_BYTES)),
            "a pinned fixture outgrew MAX_FIXTURE_BYTES"
        );
    }

    /// A catalog file is accepted by its pin, as a model or a detector of its own kind
    /// only; one byte flipped in a file of the catalog's size is refused; a file larger
    /// than any catalog file is refused before it is read.
    #[test]
    fn a_catalog_file_is_pinned() {
        use crate::catalog::{
            Languages, SttCatalogEntry, SttEngine, SttFormat, Tokenizer, STT_MODEL_CATALOG,
        };
        use sha2::{Digest, Sha256};
        let dir = tempfile::tempdir().expect("a temporary folder");
        // Larger than any fixture, so only the catalog can admit it.
        let mut bytes: Vec<u8> = (0..(5u32 << 20))
            .map(|i| (i.wrapping_mul(31) >> 3) as u8)
            .collect();
        let digest: &'static str = Box::leak(
            Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
                .into_boxed_str(),
        );
        let mut entry: SttCatalogEntry = STT_MODEL_CATALOG[0];
        entry.size_bytes = bytes.len() as u64;
        entry.sha256 = digest;
        entry.accepted_sha256 = Box::leak(vec![digest].into_boxed_slice());
        entry.languages = Languages::EnglishOnly;
        entry.engine = SttEngine::WhisperCpp;
        entry.format = SttFormat::GgmlBin;
        entry.tokenizer = Tokenizer::Gpt2;
        let catalog = Catalog {
            entries: Box::leak(vec![entry].into_boxed_slice()),
            retired: &[],
        };
        let model = dir.path().join("renamed.bin");
        std::fs::write(&model, &bytes).expect("write");
        assert!(is_pinned(&model, &[], SttKind::Speech, &catalog));
        assert!(
            !is_pinned(&model, &[], SttKind::Vad, &catalog),
            "a speech model is not a detector"
        );
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        std::fs::write(&model, &bytes).expect("write");
        assert!(!is_pinned(&model, &[], SttKind::Speech, &catalog));
        let file = std::fs::File::create(&model).expect("create");
        file.set_len(bytes.len() as u64 + 1).expect("a sparse file");
        assert!(!is_pinned(&model, &[], SttKind::Speech, &catalog));
        // The compiled catalog's detector is the detector fixture, byte for byte.
        assert!(is_pinned(
            &fixtures::path(fixtures::SILERO),
            &[],
            SttKind::Vad,
            &CATALOG
        ));
    }

    #[test]
    fn the_detector_is_not_accepted_as_a_model() {
        let (code, _, _) = run_capturing(
            &fixture_args(&fixtures::path(fixtures::SILERO)),
            cpu::host_report(),
        );
        assert_eq!(code, exit::NOT_PINNED);
    }

    #[test]
    fn usage_errors_exit_2() {
        let mut unknown = fixture_args(&fixtures::path(fixtures::BASE_EN));
        unknown.push("--verbose".into());
        let (code, out, _) = run_capturing(&unknown, cpu::host_report());
        assert_eq!(code, exit::NO_SUCH_ROLE);
        assert!(out.contains("\"stage\":\"args\""), "{out}");

        let (code, _, _) = run_capturing(&["--cpu".into()], cpu::host_report());
        assert_eq!(code, exit::NO_SUCH_ROLE);

        let mut not_a_wav = fixture_args(&fixtures::path(fixtures::BASE_EN));
        not_a_wav[5] = fixtures::path(fixtures::SILERO).into();
        let (code, out, _) = run_capturing(&not_a_wav, cpu::host_report());
        assert_eq!(code, exit::NO_SUCH_ROLE, "{out}");
    }

    #[test]
    fn the_isa_check_reads_ggml_feature_names() {
        if cfg!(target_arch = "x86_64") {
            let good = "WHISPER : COREML = 0 | OPENVINO = 0 | CPU : SSE3 = 1 | AVX = 1 | AVX2 = 1 | F16C = 1 | FMA = 1 | BMI2 = 1 | ";
            assert!(isa_missing(good).is_empty());
            let sse = "WHISPER : COREML = 0 | CPU : SSE3 = 1 | ";
            assert_eq!(isa_missing(sse), ["AVX2", "FMA", "F16C", "BMI2"]);
        }
        if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
            let good = "WHISPER : COREML = 0 | MTL : EMBED_LIBRARY = 1 | CPU : NEON = 1 | ARM_FMA = 1 | FP16_VA = 1 | DOTPROD = 1 | ";
            assert!(isa_missing(good).is_empty());
            let neon = "CPU : NEON = 1 | ARM_FMA = 1 | ";
            assert_eq!(isa_missing(neon), ["DOTPROD", "FP16_VA"]);
        }
        // The real build carries what it was configured with.
        assert!(
            isa_missing(&engine::system_info()).is_empty(),
            "{}",
            engine::system_info()
        );
    }
}
