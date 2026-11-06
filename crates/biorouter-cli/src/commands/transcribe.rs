//! `biorouter transcribe FILE|-` (PR-12; DD-47, DD-48, DD-78): a recording into text,
//! entirely on this computer, wherever the CLI runs: a terminal, an agent's shell, cron, a
//! script, a container (S8, S10, S11, S16, S19, S20).
//!
//! The order is chosen for latency. After the cheap refusals, the worker process starts
//! at once; the file is decoded on a blocking thread while the model is resolved and
//! loaded, so the load overlaps the decode, and the load skips the encoder warm up a
//! long lived worker runs, since this process decodes one recording and exits. A
//! recording that is too short or exact zeros is answered before the model is needed.
//! The recording is then cut by the voice
//! detector and every segment is decoded as soon as it is cut, with one model load for
//! all of it (`biorouter::dictation::file`). `--format json` reports where the time went.
//!
//! No dictionary yet: PR-14 adds `--session`, `--context-file` and `--no-dictionary`.
//! Until then every transcription is public only, so `bias_scope` is `"public"` and
//! `changes` is empty.
//!
//! Exit codes are those of [`crate::dictation::exit`]: 0 text, 1 a failure, 2 declined
//! to start, 3 nothing heard, 4 cancelled with Ctrl-C.

use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;

#[cfg(feature = "dictation")]
use crate::dictation::messages;
use crate::dictation::{JsonError, Outcome};

/// What `--format` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Json,
    Srt,
    Vtt,
}

impl OutputFormat {
    pub fn parse(value: &str) -> OutputFormat {
        match value {
            "json" => OutputFormat::Json,
            "srt" => OutputFormat::Srt,
            "vtt" => OutputFormat::Vtt,
            _ => OutputFormat::Text,
        }
    }
}

pub struct TranscribeArgs {
    /// A file, or `-` for standard input.
    pub input: String,
    pub format: OutputFormat,
    pub output: Option<PathBuf>,
    pub language: Option<String>,
    pub model: Option<String>,
}

/// A piece of the transcript in seconds, as the HTTP `Transcript` writes it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct JsonSegment {
    pub t0: f64,
    pub t1: f64,
    pub text: String,
}

/// One correction, as the HTTP `Transcript` writes it. Empty until PR-14.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct JsonChange {
    pub start: usize,
    pub end: usize,
    pub from: String,
    pub to: String,
    pub stage: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct JsonRetried {
    pub reason: String,
}

/// Where the time went, in milliseconds from the command's start unless named otherwise.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct JsonTimings {
    /// The command's start to the text.
    pub total_ms: u64,
    /// Reading and decoding the file to 16 kHz mono.
    pub read_ms: u64,
    /// The command's start to the model's files found and verified.
    pub resolve_ms: u64,
    /// The command's start to the model being loaded.
    pub model_ready_ms: u64,
    /// The worker's own model load and warm up, inside `model_ready_ms`.
    pub load_ms: u64,
    pub warmup_ms: u64,
    /// The recording through the detector and the decoder, after both were ready.
    pub transcribe_ms: u64,
    /// The detector's requests, and the worker's own encoder and decoder time.
    pub vad_ms: u64,
    pub encode_ms: u64,
    pub decode_ms: u64,
    pub decodes: u32,
    /// `metal` or `cpu`.
    pub backend: String,
}

/// `--format json`, schema 1: the HTTP `Transcript`'s names for every field the two share
/// (`text`, `raw_text`, `changes`, `segments`, `model`, `audio_s`, `elapsed_ms`,
/// `bias_scope`, `retried`, `no_speech`), plus `schema`, `language` and `timings`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct JsonTranscript {
    pub schema: u32,
    pub text: String,
    pub raw_text: String,
    pub changes: Vec<JsonChange>,
    pub segments: Vec<JsonSegment>,
    pub model: String,
    pub language: String,
    pub audio_s: f64,
    pub elapsed_ms: u64,
    pub bias_scope: String,
    pub retried: Option<JsonRetried>,
    pub no_speech: bool,
    pub timings: JsonTimings,
}

/// A run that ended without text: what it was, and the sentence to show.
struct Failure {
    outcome: Outcome,
    message: String,
}

impl Failure {
    fn new(outcome: Outcome, message: impl Into<String>) -> Failure {
        Failure {
            outcome,
            message: message.into(),
        }
    }
}

/// `biorouter transcribe`. Returns only with the text written; every other ending prints
/// its sentence (or, with `--format json`, a schema 1 error on stdout) and exits with its
/// code.
pub async fn handle_transcribe(args: TranscribeArgs) -> Result<()> {
    let format = args.format;
    match run(args).await {
        Ok(()) => Ok(()),
        Err(failure) => {
            if format == OutputFormat::Json {
                let error = JsonError::new(failure.outcome, failure.message);
                let mut out = std::io::stdout().lock();
                let _ = serde_json::to_writer_pretty(&mut out, &error);
                let _ = writeln!(out);
                let _ = out.flush();
            } else {
                eprintln!("{}", failure.message);
            }
            std::process::exit(failure.outcome.exit_code());
        }
    }
}

#[cfg(not(feature = "dictation"))]
async fn run(_args: TranscribeArgs) -> Result<(), Failure> {
    Err(Failure::new(
        Outcome::NotCompiled,
        biorouter::dictation::MSG_NO_ENGINE,
    ))
}

#[cfg(feature = "dictation")]
async fn run(args: TranscribeArgs) -> Result<(), Failure> {
    use std::sync::Arc;
    use std::time::Instant;

    use biorouter::dictation::service::{self, TranscribeRequest};
    use biorouter::dictation::DictationError;
    use tokio_util::sync::CancellationToken;

    use crate::dictation::decode;

    let started = Instant::now();
    if let Some(refusal) = service::spawn_refusal() {
        return Err(failure_of(refusal));
    }
    let enabled = biorouter::managed::ManagedPolicy::load()
        .dictation()
        .enabled
        .clone();
    if !enabled.is_allowed() {
        return Err(failure_of(DictationError::Policy(enabled.allowance())));
    }
    let language = match args.language.as_deref() {
        Some(language) => language.trim().to_ascii_lowercase(),
        None => service::configured_language(),
    };
    service::check_language_code(&language).map_err(failure_of)?;

    // Ctrl-C cancels the transcription and exits 4. The worker leads its own process
    // group, so the terminal's SIGINT reaches only this process.
    let cancel = CancellationToken::new();
    let on_interrupt = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            on_interrupt.cancel();
        }
    });
    let cancelled = || Failure::new(Outcome::Cancelled, messages::MSG_CANCELLED);

    // The worker process starts now; the file is read while the model loads. This process
    // loads one model for one recording, so the load skips the encoder warm up, which on
    // Metal cost 250 ms of turbo's one shot time and saved the first decode 6 to 14 ms
    // once `install` had compiled the shaders [M].
    let svc = service::service();
    svc.one_shot();
    svc.spawn_ahead().map_err(failure_of)?;
    let model = args.model.clone();
    let model_language = language.clone();
    let prepared = tokio::spawn(async move {
        // DD-32's thread count asks the llama sidecar, which took about 100 ms here [M]:
        // longer than finding the model and starting the worker. So the load does not
        // wait for it. The load uses the full count, which only the detector's warm up
        // reads, and every decode carries DD-32's count.
        let threads = tokio::spawn(svc.threads());
        let mut spec = svc
            .resolve_for(model.as_deref(), Some(&model_language))
            .await?;
        let resolved = started.elapsed();
        spec.n_threads = service::base_threads();
        let loaded = svc.prepare_with(spec.clone()).await?;
        spec.n_threads = threads.await.unwrap_or_else(|_| service::base_threads());
        Ok::<_, DictationError>((spec, loaded, resolved, started.elapsed()))
    });

    let input = args.input.clone();
    let decoding = tokio::task::spawn_blocking(move || {
        if input == "-" {
            decode::decode_reader(std::io::stdin().lock())
        } else {
            decode::decode_path(std::path::Path::new(&input))
        }
    });
    let decoded = tokio::select! {
        decoded = decoding => decoded
            .map_err(|_| Failure::new(Outcome::Unreadable, messages::msg_unreadable(&args.input)))?
            .map_err(|e| Failure::new(e.outcome(), e.message(&args.input)))?,
        () = cancel.cancelled() => return Err(cancelled()),
    };
    let read_ms = started.elapsed().as_millis() as u64;
    decoded
        .check_length()
        .map_err(|e| Failure::new(e.outcome(), e.message(&args.input)))?;
    if decoded.is_digital_silence() {
        return Err(Failure::new(Outcome::NoSpeech, messages::MSG_NO_SPEECH));
    }

    let (spec, loaded, resolved, ready) = tokio::select! {
        ready = prepared => ready
            .map_err(|_| Failure::new(Outcome::EngineFailed, messages::MSG_ENGINE_FAILED))?
            .map_err(failure_of)?,
        () = cancel.cancelled() => return Err(cancelled()),
    };
    let model_id = service::model_id(&spec);
    let pcm: Arc<[i16]> = decoded.samples.into();
    let transcribing = std::time::Instant::now();
    let request = TranscribeRequest {
        language: language.clone(),
        cancel,
    };
    let transcript = svc
        .transcribe_recording_with(spec, pcm, request)
        .await
        .map_err(failure_of)?;
    let transcribe_ms = transcribing.elapsed().as_millis() as u64;
    if transcript.no_speech {
        return Err(Failure::new(Outcome::NoSpeech, messages::MSG_NO_SPEECH));
    }
    let total_ms = started.elapsed().as_millis() as u64;
    let json = JsonTranscript {
        schema: crate::dictation::SCHEMA,
        text: transcript.text.clone(),
        raw_text: transcript.raw_text.clone(),
        changes: Vec::new(),
        segments: transcript
            .segments
            .iter()
            .map(|s| JsonSegment {
                t0: s.t0_ms as f64 / 1_000.0,
                t1: s.t1_ms as f64 / 1_000.0,
                text: s.text.clone(),
            })
            .collect(),
        model: model_id,
        language,
        audio_s: transcript.audio_ms as f64 / 1_000.0,
        elapsed_ms: total_ms,
        bias_scope: "public".into(),
        retried: transcript.retried.map(|reason| JsonRetried {
            reason: reason.as_str().into(),
        }),
        no_speech: false,
        timings: JsonTimings {
            total_ms,
            read_ms,
            resolve_ms: resolved.as_millis() as u64,
            model_ready_ms: ready.as_millis() as u64,
            load_ms: loaded.load_ms,
            warmup_ms: loaded.warmup_ms,
            transcribe_ms,
            vad_ms: transcript.timings.vad_ms,
            encode_ms: transcript.timings.encode_ms,
            decode_ms: transcript.timings.decode_ms,
            decodes: transcript.timings.decodes,
            backend: loaded.backend,
        },
    };
    let rendered = render(args.format, &json);
    match &args.output {
        Some(path) => std::fs::write(path, rendered)
            .map_err(|_| Failure::new(Outcome::Unwritable, messages::msg_unwritable(Some(path))))?,
        None => write_stdout(rendered.as_bytes(), &mut std::io::stdout().lock())?,
    }
    Ok(())
}

/// The transcript on standard output. A failed write (a full disk under `> notes.txt`)
/// exits 1 as `--output` does, so a script is never told text was written that was not.
/// A reader that closed the pipe early (`| head`) took what it wanted, so that is no
/// failure.
#[cfg(feature = "dictation")]
fn write_stdout(bytes: &[u8], out: &mut impl Write) -> Result<(), Failure> {
    match out.write_all(bytes).and_then(|()| out.flush()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(_) => Err(Failure::new(
            Outcome::Unwritable,
            messages::msg_unwritable(None),
        )),
    }
}

/// What a dictation error ends the run with.
#[cfg(feature = "dictation")]
fn failure_of(error: biorouter::dictation::DictationError) -> Failure {
    use biorouter::dictation::DictationError as E;
    let outcome = match &error {
        E::NotCompiled => Outcome::NotCompiled,
        E::UnsupportedCpu { .. } => Outcome::UnsupportedCpu,
        E::Policy(_) => Outcome::Policy,
        E::NoModel { .. } => Outcome::NoModel,
        E::NotInstalled { .. } | E::DetectorNotInstalled { .. } => Outcome::NotInstalled,
        E::UnknownModel(_) => Outcome::UnknownModel,
        E::EnglishOnly { .. } | E::LanguageUnsupported { .. } => Outcome::Language,
        E::Cancelled => Outcome::Cancelled,
        E::WorkerExited { .. }
        | E::Deadline { .. }
        | E::Protocol(_)
        | E::ModelLoadFailed { .. }
        | E::BadAudio
        | E::Internal(_) => Outcome::EngineFailed,
    };
    let message = match outcome {
        // MSG-TRANSCRIBE-FAILED: MSG-ENGINE-FAILED-CLI's first sentence, then the
        // engine's diagnostic.
        Outcome::EngineFailed => messages::msg_engine_failed(&error.to_string()),
        Outcome::Cancelled => messages::MSG_CANCELLED.to_string(),
        _ => error.to_string(),
    };
    Failure::new(outcome, message)
}

/// The transcript in `format`, ending with a newline.
pub fn render(format: OutputFormat, transcript: &JsonTranscript) -> String {
    match format {
        OutputFormat::Text => format!("{}\n", transcript.text),
        OutputFormat::Json => {
            let mut json = serde_json::to_string_pretty(transcript).unwrap_or_default();
            json.push('\n');
            json
        }
        OutputFormat::Srt => {
            let mut out = String::new();
            for (i, s) in transcript.segments.iter().enumerate() {
                out.push_str(&format!(
                    "{}\n{} --> {}\n{}\n\n",
                    i + 1,
                    clock(s.t0, ','),
                    clock(s.t1, ','),
                    s.text
                ));
            }
            out
        }
        OutputFormat::Vtt => {
            let mut out = String::from("WEBVTT\n\n");
            for s in &transcript.segments {
                out.push_str(&format!(
                    "{} --> {}\n{}\n\n",
                    clock(s.t0, '.'),
                    clock(s.t1, '.'),
                    s.text
                ));
            }
            out
        }
    }
}

/// `HH:MM:SS,mmm` (SRT) or `HH:MM:SS.mmm` (WebVTT).
fn clock(seconds: f64, separator: char) -> String {
    let ms = (seconds.max(0.0) * 1_000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}{separator}{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1_000 % 60,
        ms % 1_000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> JsonTranscript {
        JsonTranscript {
            schema: 1,
            text: "Start metformin. Review in two weeks.".into(),
            raw_text: "Start metformin. Review in two weeks.".into(),
            changes: Vec::new(),
            segments: vec![
                JsonSegment {
                    t0: 0.3,
                    t1: 2.1,
                    text: "Start metformin.".into(),
                },
                JsonSegment {
                    t0: 3661.25,
                    t1: 3663.5,
                    text: "Review in two weeks.".into(),
                },
            ],
            model: "whisper-small.en-q8_0".into(),
            language: "en".into(),
            audio_s: 3664.0,
            elapsed_ms: 912,
            bias_scope: "public".into(),
            retried: None,
            no_speech: false,
            timings: JsonTimings {
                total_ms: 912,
                read_ms: 40,
                resolve_ms: 20,
                model_ready_ms: 260,
                load_ms: 102,
                warmup_ms: 72,
                transcribe_ms: 640,
                vad_ms: 30,
                encode_ms: 400,
                decode_ms: 120,
                decodes: 2,
                backend: "metal".into(),
            },
        }
    }

    /// Schema 1 carries the HTTP transcript's names for every field the two share, and
    /// `changes` entries the names of the correction record. Checked on the serializer,
    /// because the `for-tests` model never transcribes anything.
    #[test]
    fn a_transcript_serializes_to_schema_1() {
        let value = serde_json::to_value(sample()).unwrap();
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        for key in [
            "schema",
            "text",
            "raw_text",
            "changes",
            "segments",
            "model",
            "language",
            "audio_s",
            "elapsed_ms",
            "bias_scope",
            "retried",
            "no_speech",
            "timings",
        ] {
            assert!(keys.contains(&key), "{key} is missing: {keys:?}");
        }
        assert_eq!(value["schema"], 1);
        assert_eq!(value["bias_scope"], "public");
        assert_eq!(value["segments"][1]["t0"], 3661.25);
        assert_eq!(value["retried"], serde_json::Value::Null);
        let change = JsonChange {
            start: 6,
            end: 15,
            from: "met forming".into(),
            to: "metformin".into(),
            stage: "alias".into(),
            entry_id: Some("e1".into()),
        };
        assert_eq!(
            serde_json::to_value(change).unwrap(),
            serde_json::json!({"start": 6, "end": 15, "from": "met forming", "to": "metformin",
                               "stage": "alias", "entry_id": "e1"})
        );
        assert_eq!(
            serde_json::to_value(JsonRetried {
                reason: "collapse".into()
            })
            .unwrap(),
            serde_json::json!({"reason": "collapse"})
        );
    }

    #[test]
    fn subtitles_carry_the_segments_times() {
        let t = sample();
        assert_eq!(
            render(OutputFormat::Srt, &t),
            "1\n00:00:00,300 --> 00:00:02,100\nStart metformin.\n\n2\n01:01:01,250 --> 01:01:03,500\nReview in two weeks.\n\n"
        );
        assert_eq!(
            render(OutputFormat::Vtt, &t),
            "WEBVTT\n\n00:00:00.300 --> 00:00:02.100\nStart metformin.\n\n01:01:01.250 --> 01:01:03.500\nReview in two weeks.\n\n"
        );
        assert_eq!(
            render(OutputFormat::Text, &t),
            "Start metformin. Review in two weeks.\n"
        );
        assert!(render(OutputFormat::Json, &t).starts_with("{\n  \"schema\": 1,"));
        assert_eq!(OutputFormat::parse("vtt"), OutputFormat::Vtt);
        assert_eq!(OutputFormat::parse("text"), OutputFormat::Text);
    }

    /// Standard output that fails is `unwritable` (exit 1), as `--output` is; a pipe its
    /// reader closed is not a failure.
    #[cfg(feature = "dictation")]
    #[test]
    fn a_failed_write_to_standard_output_exits_1() {
        struct Fails(std::io::ErrorKind);
        impl Write for Fails {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(self.0.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let failure = write_stdout(b"text\n", &mut Fails(std::io::ErrorKind::Other))
            .expect_err("a full disk is a failure");
        assert_eq!(failure.outcome, Outcome::Unwritable);
        assert_eq!(failure.outcome.exit_code(), 1);
        assert_eq!(
            failure.message,
            "The transcript could not be written to standard output."
        );
        assert!(write_stdout(b"text\n", &mut Fails(std::io::ErrorKind::BrokenPipe)).is_ok());
        let mut written = Vec::new();
        assert!(write_stdout(b"text\n", &mut written).is_ok());
        assert_eq!(written, b"text\n");
    }
}
