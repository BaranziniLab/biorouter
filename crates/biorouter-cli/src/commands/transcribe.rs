//! `biorouter transcribe FILE|-` (PR-12; DD-47, DD-48, DD-78): a recording into text,
//! entirely on this computer, wherever the CLI runs: a terminal, an agent's shell, cron, a
//! script, a container (S8, S10, S11, S16, S19, S20).
//!
//! The order is chosen for latency. After the cheap refusals, the worker process starts
//! at once; the file is decoded on a blocking thread while the model is resolved and
//! loaded, so the load overlaps the decode (a compressed M4A, AAC, MP3 or Ogg file
//! included, PR-12b), and the load skips the encoder warm up a
//! long lived worker runs, since this process decodes one recording and exits. A
//! recording that is too short or exact zeros is answered before the model is needed.
//! The recording is then cut by the voice
//! detector and every segment is decoded as soon as it is cut, with one model load for
//! all of it (`biorouter::dictation::file`). `--format json` reports where the time went.
//!
//! The personal dictionary shapes the transcript (PR-14, DD-36, DD-77): its terms bias the
//! decoder through the token prompt (the decoder bonus is built but off, M-22), and
//! correction rewrites what was misheard, every change listed in `changes`. Which terms
//! may be used follows DD-77: public terms only, unless `--session` names a private chat,
//! the run is at an interactive terminal, the configured model is private and
//! `BIOROUTER_TERMINAL` is unset, so an agent's shell never gets a private term whatever
//! its flags. `--context-file` ranks the terms (at most 4 KB of it is read, and it goes
//! nowhere); `--no-dictionary` turns all of it off and prepares nothing, not even the word
//! list, and the managed `dictionary: false` key does the same. The dictionary is prepared
//! while the model loads, so none of it waits for the decode.
//!
//! Once the text is written, and only then, the terms the text holds have their `uses`
//! raised in `usage.json` ("Usage"): one batch, the store's lock tried once, the file
//! replaced with no flush to disk. On Unix standard output is pointed at the null device
//! first, so a pipe's reader has the whole text before that write starts; only a caller
//! that waits for the exit waits for it. An agent's shell raises none, as it writes nothing
//! else in the dictionary.
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
    /// `--session`: the chat the transcript is for (DD-77).
    pub session: Option<String>,
    /// `--context-file`: text that ranks the dictionary's terms.
    pub context_file: Option<PathBuf>,
    /// `--no-dictionary`.
    pub no_dictionary: bool,
}

/// A piece of the transcript in seconds, as the HTTP `Transcript` writes it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct JsonSegment {
    pub t0: f64,
    pub t1: f64,
    pub text: String,
}

/// One correction, as the HTTP `Transcript` writes it: `start` and `end` count characters
/// of `text` and bound `to`; `stage` is `alias`, `exact` or `fuzzy`.
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
    /// The command's start to the dictionary prepared: the bias set, its ranking, the
    /// corrector and the bonus table's terms, all while the model loads.
    pub dictionary_ms: u64,
    /// Correction of the decoded text, in microseconds: the dictionary's share of the time
    /// after the last decode.
    pub correct_us: u64,
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
    use tokio_util::sync::CancellationToken;

    let started = Instant::now();
    let Admitted {
        agent_shell,
        records_usage,
        no_dictionary,
        language,
    } = admit(&args)?;

    // Ctrl-C cancels the transcription and exits 4. The worker leads its own process
    // group, so the terminal's SIGINT reaches only this process.
    let cancel = CancellationToken::new();
    let on_interrupt = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            on_interrupt.cancel();
        }
    });

    // The worker process starts now; the file is read while the model loads. This process
    // loads one model for one recording, so the load skips the encoder warm up, which on
    // Metal cost 250 ms of turbo's one shot time and saved the first decode 6 to 14 ms
    // once `install` had compiled the shaders [M].
    let svc = service::service();
    svc.one_shot();
    svc.spawn_ahead().map_err(failure_of)?;
    let prepared = start_model(svc, args.model.clone(), language.clone(), started);
    let dictionary = start_dictionary(&args, svc, no_dictionary, agent_shell, started)?;
    let (decoded, read_ms) = read_recording(&args.input, &cancel, started).await?;

    let (spec, loaded, resolved, ready) = tokio::select! {
        ready = prepared => ready
            .map_err(|_| Failure::new(Outcome::EngineFailed, messages::MSG_ENGINE_FAILED))?
            .map_err(failure_of)?,
        () = cancel.cancelled() => return Err(cancelled()),
    };
    let model_id = service::model_id(&spec);
    let (dictionary, dictionary_ready) = tokio::select! {
        ready = dictionary => ready
            .map_err(|_| Failure::new(Outcome::EngineFailed, messages::MSG_ENGINE_FAILED))?,
        () = cancel.cancelled() => return Err(cancelled()),
    };
    let pcm: Arc<[i16]> = decoded.samples.into();
    let transcribing = std::time::Instant::now();
    let request = TranscribeRequest {
        language: language.clone(),
        cancel,
        dictionary: dictionary.clone(),
        decode: None,
    };
    let dictated = svc
        .transcribe_recording_with(spec, pcm, request)
        .await
        .map_err(failure_of)?;
    let transcribe_ms = transcribing.elapsed().as_millis() as u64;
    let transcript = &dictated.transcript;
    if transcript.no_speech {
        return Err(Failure::new(Outcome::NoSpeech, messages::MSG_NO_SPEECH));
    }
    let total_ms = started.elapsed().as_millis() as u64;
    let timings = JsonTimings {
        dictionary_ms: dictionary_ready.as_millis() as u64,
        correct_us: dictated.correct_us,
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
    };
    let json = transcript_json(&dictated, model_id, language, total_ms, timings);
    let rendered = render(args.format, &json);
    match &args.output {
        Some(path) => std::fs::write(path, rendered)
            .map_err(|_| Failure::new(Outcome::Unwritable, messages::msg_unwritable(Some(path))))?,
        None => {
            write_stdout(rendered.as_bytes(), &mut std::io::stdout().lock())?;
            #[cfg(unix)]
            close_stdout();
        }
    }
    // The text is out, and a reader of the pipe has seen its end. Only now does the
    // dictionary learn which of its terms it used, so the write costs nothing between stop
    // and text.
    if let Some(dictionary) = dictionary.filter(|_| records_usage) {
        let text = transcript.text.clone();
        let _ = tokio::task::spawn_blocking(move || record_uses(&dictionary, &text)).await;
    }
    Ok(())
}

/// What the refusals before the worker starts leave for the run.
#[cfg(feature = "dictation")]
struct Admitted {
    /// `BIOROUTER_TERMINAL`'s value, read once (DD-77).
    agent_shell: Option<std::ffi::OsString>,
    /// Not an agent's shell, so the terms the text holds have their `uses` raised.
    records_usage: bool,
    /// `--no-dictionary`, or the managed `dictionary: false`.
    no_dictionary: bool,
    language: String,
}

/// The refusals before the worker starts: a processor the engine cannot run on, the
/// managed `enabled: false`, and a language code no model knows.
#[cfg(feature = "dictation")]
fn admit(args: &TranscribeArgs) -> Result<Admitted, Failure> {
    use biorouter::dictation::service;
    use biorouter::dictation::DictationError;

    if let Some(refusal) = service::spawn_refusal() {
        return Err(failure_of(refusal));
    }
    // DD-77 reads this once: it decides the bias target and whether usage is recorded.
    let agent_shell = std::env::var_os(super::dictation::AGENT_SHELL_ENV);
    let records_usage = !is_agent_shell(agent_shell.as_deref());
    // The managed policy is read once: `enabled` refuses the run, and `dictionary: false`
    // runs it as `--no-dictionary` does, so no store is read and no usage is written.
    let policy = biorouter::managed::ManagedPolicy::load();
    let enabled = &policy.dictation().enabled;
    if !enabled.is_allowed() {
        return Err(failure_of(DictationError::Policy(enabled.allowance())));
    }
    let no_dictionary = without_dictionary(args.no_dictionary, &policy.dictation().dictionary);
    let language = match args.language.as_deref() {
        Some(language) => language.trim().to_ascii_lowercase(),
        None => service::configured_language(),
    };
    service::check_language_code(&language).map_err(failure_of)?;
    Ok(Admitted {
        agent_shell,
        records_usage,
        no_dictionary,
        language,
    })
}

/// MSG-CANCELLED: Ctrl-C ended the run (exit 4).
#[cfg(feature = "dictation")]
fn cancelled() -> Failure {
    Failure::new(Outcome::Cancelled, messages::MSG_CANCELLED)
}

/// The model found and loaded on its own task: the spec with DD-32's thread count for
/// every decode, the load, and when the model was found and when it was ready, both
/// counted from `started`.
#[cfg(feature = "dictation")]
fn start_model(
    svc: &'static biorouter::dictation::DictationService,
    model: Option<String>,
    model_language: String,
    started: std::time::Instant,
) -> tokio::task::JoinHandle<Result<PreparedModel, biorouter::dictation::DictationError>> {
    use biorouter::dictation::service;
    use biorouter::dictation::DictationError;

    tokio::spawn(async move {
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
    })
}

/// What [`start_model`] gives: the spec, the load, and when the model was found and
/// when it was ready.
#[cfg(feature = "dictation")]
type PreparedModel = (
    biorouter::dictation::LoadSpec,
    biorouter::dictation::Loaded,
    std::time::Duration,
    std::time::Duration,
);

/// The dictionary, prepared on its own task while the model loads and the file decodes:
/// the bias set under DD-77, its ranking against the context and the corrector (and the
/// decoder bonus table, when measurement turns the bonus on, which the worker builds right
/// after the model loads). `--no-dictionary` and the managed `dictionary: false` prepare
/// nothing. A `--context-file` that cannot be read ends the run first.
#[cfg(feature = "dictation")]
fn start_dictionary(
    args: &TranscribeArgs,
    svc: &'static biorouter::dictation::DictationService,
    no_dictionary: bool,
    agent_shell: Option<std::ffi::OsString>,
    started: std::time::Instant,
) -> Result<PreparedDictionary, Failure> {
    use std::sync::Arc;

    let context = match &args.context_file {
        Some(path) => Some(read_context(path)?),
        None => None,
    };
    let session = args.session.clone();
    Ok(tokio::spawn(async move {
        let request = prepare_dictionary(no_dictionary, session, context, agent_shell)
            .await
            .map(Arc::new);
        if let Some(request) = &request {
            let _ = svc.install_bias(request);
        }
        (request, started.elapsed())
    }))
}

/// What [`start_dictionary`] gives: the dictionary, and when it was ready.
#[cfg(feature = "dictation")]
type PreparedDictionary = tokio::task::JoinHandle<(
    Option<std::sync::Arc<biorouter::dictation::DictionaryRequest>>,
    std::time::Duration,
)>;

/// The recording, decoded to 16 kHz mono on a blocking thread while the model loads, and
/// when that finished (from `started`); Ctrl-C cancels the wait. A recording under half a
/// second, or exact zeros, is answered here, before the model is needed.
#[cfg(feature = "dictation")]
async fn read_recording(
    input: &str,
    cancel: &tokio_util::sync::CancellationToken,
    started: std::time::Instant,
) -> Result<(crate::dictation::decode::Decoded, u64), Failure> {
    use crate::dictation::decode;

    let source = input.to_string();
    let decoding = tokio::task::spawn_blocking(move || {
        if source == "-" {
            decode::decode_reader(std::io::stdin().lock())
        } else {
            decode::decode_path(std::path::Path::new(&source))
        }
    });
    let decoded = tokio::select! {
        decoded = decoding => decoded
            .map_err(|_| Failure::new(Outcome::Unreadable, messages::msg_unreadable(input)))?
            .map_err(|e| Failure::new(e.outcome(), e.message(input)))?,
        () = cancel.cancelled() => return Err(cancelled()),
    };
    let read_ms = started.elapsed().as_millis() as u64;
    decoded
        .check_length()
        .map_err(|e| Failure::new(e.outcome(), e.message(input)))?;
    if decoded.is_digital_silence() {
        return Err(Failure::new(Outcome::NoSpeech, messages::MSG_NO_SPEECH));
    }
    Ok((decoded, read_ms))
}

/// `--format json`'s object for a transcription, which every format renders.
#[cfg(feature = "dictation")]
fn transcript_json(
    dictated: &biorouter::dictation::DictatedRecording,
    model: String,
    language: String,
    elapsed_ms: u64,
    timings: JsonTimings,
) -> JsonTranscript {
    let transcript = &dictated.transcript;
    JsonTranscript {
        schema: crate::dictation::SCHEMA,
        text: transcript.text.clone(),
        raw_text: transcript.raw_text.clone(),
        changes: dictated
            .changes
            .iter()
            .map(|c| JsonChange {
                start: c.start,
                end: c.end,
                from: c.from.clone(),
                to: c.to.clone(),
                stage: c.stage.as_str().to_string(),
                entry_id: Some(c.entry_id.clone()),
            })
            .collect(),
        segments: transcript
            .segments
            .iter()
            .map(|s| JsonSegment {
                t0: s.t0_ms as f64 / 1_000.0,
                t1: s.t1_ms as f64 / 1_000.0,
                text: s.text.clone(),
            })
            .collect(),
        model,
        language: reported_language(language, transcript.language.as_deref()),
        audio_s: transcript.audio_ms as f64 / 1_000.0,
        elapsed_ms,
        bias_scope: dictated.bias_scope.as_str().into(),
        retried: transcript.retried.map(|reason| JsonRetried {
            reason: reason.as_str().into(),
        }),
        no_speech: false,
        timings,
    }
}

/// The `language` of `--format json`: the code the run asked for, or, for `auto`, the one
/// the model detected for most of the speech. A worker that reports none (one built before
/// PR-12b) leaves `auto`.
#[cfg(feature = "dictation")]
fn reported_language(asked: String, detected: Option<&str>) -> String {
    match detected {
        Some(code) if asked == "auto" => code.to_string(),
        _ => asked,
    }
}

/// Whether this run uses no dictionary: `--no-dictionary`, or the managed `dictionary:
/// false` key ("Managed policy keys"), which turns off the dictionary's bias and so its
/// usage too. `bias_set` would give such a run the empty set anyway; deciding here also
/// skips reading the store.
#[cfg(feature = "dictation")]
fn without_dictionary(flag: bool, dictionary: &biorouter::managed::Rule) -> bool {
    flag || !dictionary.is_allowed()
}

/// Points standard output at the null device once the text is written and flushed, so a
/// reader of a pipe (`biorouter transcribe a.wav | pbcopy`) sees the end of the text now,
/// not when the process exits after recording usage. Nothing after the text writes to
/// standard output, and the worker's output is its own pipe. If the null device cannot be
/// opened, standard output stays open until the exit.
#[cfg(all(feature = "dictation", unix))]
fn close_stdout() {
    use std::os::fd::AsRawFd;
    if let Ok(null) = std::fs::OpenOptions::new().write(true).open("/dev/null") {
        // SAFETY: both descriptors are open; `dup2` replaces descriptor 1 atomically, and
        // `write_stdout` flushed what Rust's standard output had buffered.
        unsafe {
            libc::dup2(null.as_raw_fd(), libc::STDOUT_FILENO);
        }
    }
}

/// Whether `BIOROUTER_TERMINAL` marks an agent's shell: any value but an empty one, as for
/// `dictation` and `dictionary`.
#[cfg(feature = "dictation")]
fn is_agent_shell(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|v| !v.is_empty())
}

/// Raises `uses` of the terms `text` holds in `usage.json`, one batch under the store's
/// lock ("Usage"). Called after the text is written. The lock is tried once: a one shot
/// process would otherwise hold its exit for the lock, and a lost bump is acceptable.
/// Returns whether a batch was written.
#[cfg(feature = "dictation")]
fn record_uses(dictionary: &biorouter::dictation::DictionaryRequest, text: &str) -> bool {
    use biorouter::security::dictionary_store::{self, store};

    let bumps = dictionary.usage_bumps(text, store::schema::now());
    biorouter::dictation::record_usage(
        &dictionary_store::open_store(),
        &bumps,
        std::time::Duration::ZERO,
    )
}

/// `--context-file`, at most [`MAX_CONTEXT_BYTES`] of it, cut at a character boundary.
/// The context only ranks the dictionary's terms; it never reaches the prompt or a log.
///
/// [`MAX_CONTEXT_BYTES`]: biorouter::dictation::recognize::select::MAX_CONTEXT_BYTES
#[cfg(feature = "dictation")]
fn read_context(path: &std::path::Path) -> Result<String, Failure> {
    use std::io::Read;

    use biorouter::dictation::recognize::select::{clip_context, MAX_CONTEXT_BYTES};

    let shown = path.display().to_string();
    let file = std::fs::File::open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            Failure::new(Outcome::NotFound, messages::msg_not_found(&shown))
        }
        _ => Failure::new(Outcome::Unreadable, messages::msg_unreadable(&shown)),
    })?;
    let mut bytes = Vec::with_capacity(MAX_CONTEXT_BYTES);
    file.take(MAX_CONTEXT_BYTES as u64 + 4)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::new(Outcome::Unreadable, messages::msg_unreadable(&shown)))?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(clip_context(&text).to_string())
}

/// The dictionary for this transcription (DD-36, DD-77), or `None` for `--no-dictionary`,
/// which reads no store, decodes no word list and builds no corrector. Public terms only,
/// unless `--session` names a private chat, the run is at an interactive terminal, the
/// configured model is private and `BIOROUTER_TERMINAL` (`agent_shell`) is unset. The
/// chat's row and the configured model are read only when everything else would admit
/// private terms, and a chat that cannot be read leaves the transcription public, whatever
/// the reason, so the answer says nothing about which chats exist. A store that cannot be
/// read gives no terms.
#[cfg(feature = "dictation")]
async fn prepare_dictionary(
    no_dictionary: bool,
    session: Option<String>,
    context: Option<String>,
    agent_shell: Option<std::ffi::OsString>,
) -> Option<biorouter::dictation::DictionaryRequest> {
    use biorouter::dictation::recognize::EligibleEntries;
    use biorouter::dictation::{bias_set, cli_bias, BiasChannels, DictionaryRequest};
    use biorouter::security::dictionary_store;

    if no_dictionary {
        return None;
    }
    let terminal = sample_terminal(
        session,
        agent_shell,
        super::needs_terminal::prompt_can_run(),
        || async {
            crate::session::privacy::caller_capability()
                .await
                .is_private()
        },
        |id| async move {
            biorouter::session::SessionManager::instance()
                .get_session(&id, false)
                .await
                .ok()
                .map(|s| s.privacy_tier)
        },
    )
    .await;
    let (target, standing) = cli_bias(terminal);
    let enforced = biorouter::privacy::privacy_tiers_enabled();
    let context = context.unwrap_or_default();
    let channels = BiasChannels::for_dictation();
    let request = tokio::task::spawn_blocking(move || {
        let eligible = match dictionary_store::open_store().view() {
            Ok(view) => bias_set(&view, &target, standing, enforced),
            Err(_) => EligibleEntries::empty(),
        };
        DictionaryRequest::new(eligible, &context, channels)
    })
    .await
    .unwrap_or_else(|_| DictionaryRequest::none());
    Some(request)
}

/// What DD-77 reads for one transcription, each sampled once: `agent_shell` is
/// `BIOROUTER_TERMINAL`'s value (any value but an empty one is an agent's shell, as for
/// `dictation` and `dictionary`), `interactive` is `prompt_can_run()`. The configured model
/// is asked (`caller_private`) only when a `--session` was given, a person is at the
/// terminal and this is not an agent's shell, and the chat's stored tier (`chat_tier`,
/// `None` when its row cannot be read) only when that model is private. A function of its
/// inputs, so the rule's wiring is tested without a terminal, a chat or a model.
#[cfg(feature = "dictation")]
async fn sample_terminal<C, CF, S, SF>(
    session: Option<String>,
    agent_shell: Option<std::ffi::OsString>,
    interactive: bool,
    caller_private: C,
    chat_tier: S,
) -> biorouter::dictation::CliTerminal
where
    C: FnOnce() -> CF,
    CF: std::future::Future<Output = bool>,
    S: FnOnce(String) -> SF,
    SF: std::future::Future<Output = Option<biorouter::privacy::SessionClassification>>,
{
    let mut terminal = biorouter::dictation::CliTerminal {
        session: None,
        interactive,
        caller_private: false,
        agent_shell: agent_shell.is_some_and(|v| !v.is_empty()),
    };
    if let Some(id) = session.filter(|_| terminal.interactive && !terminal.agent_shell) {
        terminal.caller_private = caller_private().await;
        if terminal.caller_private {
            terminal.session = chat_tier(id).await;
        }
    }
    terminal
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
                dictionary_ms: 30,
                correct_us: 90,
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

    /// `--no-dictionary` prepares nothing: no store read, no word list, no corrector, so
    /// the transcript carries no dictionary at all and its usage records nothing.
    #[cfg(feature = "dictation")]
    #[test]
    fn no_dictionary_prepares_nothing() {
        let prepared = futures::executor::block_on(prepare_dictionary(
            true,
            Some("20261106_1".into()),
            Some("EPAS1 and HIF-2α".into()),
            None,
        ));
        assert!(prepared.is_none());
        use std::ffi::OsStr;
        assert!(!is_agent_shell(None));
        assert!(!is_agent_shell(Some(OsStr::new(""))));
        for value in ["1", "0", "true"] {
            assert!(is_agent_shell(Some(OsStr::new(value))), "{value}");
        }
    }

    /// `--language auto` reports the language the model detected; a code that was asked
    /// for is reported as asked, and an older worker's result, which carries none, leaves
    /// `auto`.
    #[cfg(feature = "dictation")]
    #[test]
    fn auto_reports_the_detected_language() {
        assert_eq!(reported_language("auto".into(), Some("de")), "de");
        assert_eq!(reported_language("auto".into(), None), "auto");
        assert_eq!(reported_language("en".into(), Some("en")), "en");
        assert_eq!(reported_language("fr".into(), None), "fr");
        assert_eq!(reported_language("fr".into(), Some("en")), "fr");
    }

    /// The managed `dictionary: false` key runs `transcribe` as `--no-dictionary` does: no
    /// dictionary is prepared, so none biases the decode or corrects the text, and the run
    /// has nothing to raise in `usage.json` (only a prepared dictionary records usage).
    /// The rule comes from the policy that turns dictation off, which forbids every key.
    #[cfg(feature = "dictation")]
    #[test]
    fn a_forbidden_dictionary_prepares_nothing_and_records_nothing() {
        use biorouter::managed::{DictationPolicy, Rule};

        let forbidden = DictationPolicy::off().dictionary;
        assert!(!forbidden.is_allowed());
        assert!(without_dictionary(false, &forbidden));
        assert!(without_dictionary(true, &forbidden));
        assert!(without_dictionary(true, &Rule::Allowed));
        assert!(!without_dictionary(false, &Rule::Allowed));
        assert!(!without_dictionary(
            false,
            &DictationPolicy::unmanaged().dictionary
        ));
        let prepared = futures::executor::block_on(prepare_dictionary(
            without_dictionary(false, &forbidden),
            None,
            Some("EPAS1".into()),
            None,
        ));
        assert!(prepared.is_none(), "nothing prepared, so nothing recorded");
    }

    /// DD-77's wiring, from the sampled inputs to the bias target and standing: with
    /// `BIOROUTER_TERMINAL` set, a run at a terminal, for a private chat, with a private
    /// model configured, is public, and neither the model nor the chat is even read. The
    /// same run without it, or with it empty, is private, so the variable alone separates
    /// the two.
    #[cfg(feature = "dictation")]
    #[test]
    fn an_agent_shell_samples_to_a_public_target() {
        use std::cell::Cell;
        use std::ffi::OsString;

        use biorouter::dictation::cli_bias;
        use biorouter::privacy::SessionClassification;

        let private_chat = Some(SessionClassification::Private);
        let run = |agent_shell: Option<&str>,
                   session: Option<&str>,
                   interactive: bool,
                   caller_private: bool,
                   chat: Option<SessionClassification>| {
            let asked = Cell::new((false, false));
            let terminal = futures::executor::block_on(sample_terminal(
                session.map(str::to_string),
                agent_shell.map(OsString::from),
                interactive,
                || {
                    asked.set((true, asked.get().1));
                    std::future::ready(caller_private)
                },
                |id| {
                    assert_eq!(id, "20261106_1");
                    asked.set((asked.get().0, true));
                    std::future::ready(chat)
                },
            ));
            let (target, standing) = cli_bias(terminal);
            assert_eq!(target.private, standing.is_private());
            (target.private, asked.get())
        };
        let id = Some("20261106_1");
        // Every condition met: private, and both the model and the chat were read.
        assert_eq!(
            run(None, id, true, true, private_chat),
            (true, (true, true))
        );
        assert_eq!(
            run(Some(""), id, true, true, private_chat),
            (true, (true, true)),
            "an empty value is unset"
        );
        // An agent's shell: public, and nothing else was read.
        for value in ["1", "true", "0"] {
            assert_eq!(
                run(Some(value), id, true, true, private_chat),
                (false, (false, false)),
                "BIOROUTER_TERMINAL={value}"
            );
        }
        // No terminal, no `--session`: public, nothing read.
        assert_eq!(
            run(None, id, false, true, private_chat),
            (false, (false, false))
        );
        assert_eq!(
            run(None, None, true, true, private_chat),
            (false, (false, false))
        );
        // A public model: the chat is never read. A chat that cannot be read, or a public
        // one, leaves the run public.
        assert_eq!(
            run(None, id, true, false, private_chat),
            (false, (true, false))
        );
        assert_eq!(run(None, id, true, true, None), (false, (true, true)));
        assert_eq!(
            run(None, id, true, true, Some(SessionClassification::Public)),
            (false, (true, true))
        );
    }
}
