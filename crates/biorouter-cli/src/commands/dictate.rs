//! `biorouter dictate` (PR-25; DD-73, DD-76, DD-77, DD-86): the terminal's microphone, the
//! engine in this process, and the text on stdout or the clipboard, reviewed first.
//!
//! It runs only for a person at their own terminal: in an agent's shell
//! (`BIOROUTER_TERMINAL` set) or without a terminal it refuses before anything starts, as
//! the detection order of `docs/dictation/surfaces.md` ("The terminal") has it, and a host
//! with no reachable microphone (SSH, a container, WSL without WSLg, a server) answers with
//! the routes that work there before any model is asked for.
//!
//! The order is chosen for latency (owner, 2026-11-05). After the refusals that need
//! nothing, the capture child and the worker process start together; the capture child's
//! probe, the model's resolution and the load run side by side; the model loads (with its
//! warm up, which on Metal compiles any shader the cache lacks) while the person speaks;
//! the dictionary is prepared while they speak; and segments the person has finished are
//! decoded while they go on speaking ([`LiveDecode`]), so a stop leaves only the open tail
//! to decode. The text is shown for review on the terminal (Enter uses it, `e` edits it,
//! Escape discards it), so `"$(biorouter dictate)"` and pipes send nothing the person has
//! not seen; an Enter that was already waiting when the text appeared does not count.
//! `--no-review` (or `--yes`) skips the review. Only the text, or with `--format json` one
//! schema 1 object, goes to stdout.
//!
//! Which dictionary entries may bias it (DD-77): public ones only, unless the person passes
//! `--private-terms` at their own terminal with a private model configured and answers `y`
//! to MSG-PRIVATE-TERMS (O-8), or `--session` names a private chat under the same rule as
//! `transcribe`.
//!
//! Exit codes: 0 text, 1 a failure, 2 declined to start, 3 nothing heard, 4 cancelled, 5
//! `--copy` found no clipboard and printed the text.

use std::io::Write;

use anyhow::Result;
use serde::Serialize;

use crate::commands::transcribe::{JsonChange, JsonRetried, JsonSegment};
use crate::dictation::{messages, JsonError, Outcome};

/// What `--format` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DictateFormat {
    Text,
    Json,
}

impl DictateFormat {
    pub fn parse(value: &str) -> DictateFormat {
        if value == "json" {
            DictateFormat::Json
        } else {
            DictateFormat::Text
        }
    }
}

/// `biorouter dictate`'s arguments.
pub struct DictateArgs {
    pub copy: bool,
    pub no_history: bool,
    pub format: DictateFormat,
    /// At most 600; a cap, never a way around the terminal check.
    pub seconds: Option<u32>,
    pub session: Option<String>,
    pub private_terms: bool,
    pub language: Option<String>,
    pub model: Option<String>,
    pub no_dictionary: bool,
    /// `--no-review` or `--yes`.
    pub no_review: bool,
    /// The hidden `--file PATH`, an alias of `transcribe PATH`.
    pub file: Option<String>,
}

/// Where the time went, in milliseconds from the command's start unless named otherwise.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DictateTimings {
    /// The capture child's probe.
    pub probe_ms: u64,
    /// The model's files found and verified.
    pub resolve_ms: u64,
    /// The device open and streaming (`ready`).
    pub device_ready_ms: u64,
    /// The first audio frame.
    pub first_audio_ms: u64,
    /// The model loaded (and warmed up), counted from the command's start.
    pub model_ready_ms: u64,
    /// The worker's own load and warm up.
    pub load_ms: u64,
    pub warmup_ms: u64,
    /// From the stop to the child's last audio frame.
    pub stop_to_last_frame_ms: u64,
    /// From the stop to the text, before the review: the latency a person feels.
    pub stop_to_text_ms: u64,
    /// From the stop to the tail's last decode.
    pub tail_ms: u64,
    pub tail_audio_s: f64,
    pub segments: u32,
    /// Segments decoded while the person was still speaking.
    pub segments_while_speaking: u32,
    pub encode_ms: u64,
    pub decode_ms: u64,
    /// Correction, in microseconds.
    pub correct_us: u64,
    /// The device's own rate, which miniaudio resampled from.
    pub device_rate: u32,
    pub backend: String,
}

/// `--format json`, schema 1: the names `transcribe` uses for every field the two share.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DictateJson {
    pub schema: u32,
    pub text: String,
    pub raw_text: String,
    pub changes: Vec<JsonChange>,
    pub segments: Vec<JsonSegment>,
    pub model: String,
    pub language: String,
    pub audio_s: f64,
    /// From the stop to the text.
    pub elapsed_ms: u64,
    pub bias_scope: String,
    pub retried: Option<JsonRetried>,
    pub no_speech: bool,
    /// Where the text went: `stdout` or `clipboard`.
    pub destination: String,
    pub timings: DictateTimings,
}

/// A run that ended without its text: what it was, and the sentences to show.
pub struct Failure {
    pub outcome: Outcome,
    pub lines: Vec<String>,
    /// MSG-NO-CLIPBOARD's text, printed on stdout after the sentence (exit 5).
    pub stdout: Option<String>,
}

impl Failure {
    pub fn new(outcome: Outcome, line: impl Into<String>) -> Failure {
        Failure {
            outcome,
            lines: vec![line.into()],
            stdout: None,
        }
    }

    pub fn lines(outcome: Outcome, lines: Vec<String>) -> Failure {
        Failure {
            outcome,
            lines,
            stdout: None,
        }
    }
}

/// `biorouter dictate`. Returns only with the text out; every other ending prints its
/// sentences (or, with `--format json`, a schema 1 error on stdout) and exits with its code.
pub async fn handle_dictate(args: DictateArgs) -> Result<()> {
    if let Some(input) = args.file.clone() {
        use crate::commands::transcribe::{handle_transcribe, OutputFormat, TranscribeArgs};
        return handle_transcribe(TranscribeArgs {
            input,
            format: match args.format {
                DictateFormat::Json => OutputFormat::Json,
                DictateFormat::Text => OutputFormat::Text,
            },
            output: None,
            language: args.language,
            model: args.model,
            session: args.session,
            context_file: None,
            no_dictionary: args.no_dictionary,
        })
        .await;
    }
    let format = args.format;
    match run(args).await {
        Ok(()) => Ok(()),
        Err(failure) => {
            report(&failure, format);
            std::process::exit(failure.outcome.exit_code());
        }
    }
}

fn report(failure: &Failure, format: DictateFormat) {
    if format == DictateFormat::Json && failure.stdout.is_none() {
        let error = JsonError::new(failure.outcome, failure.lines.join(" "));
        let mut out = std::io::stdout().lock();
        let _ = serde_json::to_writer_pretty(&mut out, &error);
        let _ = writeln!(out);
        let _ = out.flush();
        return;
    }
    for line in &failure.lines {
        eprintln!("{line}");
    }
    if let Some(text) = &failure.stdout {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{text}");
        let _ = out.flush();
    }
}

#[cfg(not(feature = "dictation"))]
async fn run(_args: DictateArgs) -> Result<(), Failure> {
    Err(Failure::new(
        Outcome::NotCompiled,
        biorouter::dictation::MSG_NO_ENGINE,
    ))
}

/// `biorouter dictation test [--seconds N]`: the first run path. It opens the microphone
/// for `seconds` (5 by default), shows the level, and reports what arrived: the device, how
/// many samples against the wall clock, how soon the first one came, and the level. Exact
/// zeros give the permission sentence (exit 3). Nothing is transcribed and nothing is kept.
pub async fn handle_microphone_test(seconds: u32) -> Result<()> {
    #[cfg(feature = "dictation")]
    let result = live::microphone_test(seconds).await;
    #[cfg(not(feature = "dictation"))]
    let result: Result<(), Failure> = {
        let _ = seconds;
        Err(Failure::new(
            Outcome::NotCompiled,
            biorouter::dictation::MSG_NO_ENGINE,
        ))
    };
    match result {
        Ok(()) => Ok(()),
        Err(failure) => {
            report(&failure, DictateFormat::Text);
            std::process::exit(failure.outcome.exit_code());
        }
    }
}

/// What `dictation test` measured.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TestRecording {
    pub device: String,
    pub backend: String,
    pub device_rate: u32,
    pub samples: u64,
    /// From `ready` to `stopped`.
    pub wall: std::time::Duration,
    /// From the `start` request to the first audio frame.
    pub first_audio_ms: u64,
    pub peak: f32,
    pub rms: f32,
}

/// The lines `dictation test` prints.
pub fn test_report(t: &TestRecording) -> Vec<String> {
    let TestRecording {
        device,
        backend,
        device_rate,
        first_audio_ms,
        peak,
        rms,
        ..
    } = t;
    let (peak, rms) = (*peak, *rms);
    let seconds = t.samples as f64 / 16_000.0;
    let wall_s = t.wall.as_secs_f64();
    let share = if wall_s > 0.0 {
        seconds / wall_s * 100.0
    } else {
        0.0
    };
    let dbfs = |v: f32| {
        if v > 0.0 {
            format!("{:.0} dBFS", 20.0 * f64::from(v).log10())
        } else {
            "silence".to_string()
        }
    };
    vec![
        format!("Microphone: {device} ({backend}, {device_rate} Hz, delivered at 16000 Hz)"),
        format!(
            "Recorded {seconds:.1} s of audio in {wall_s:.2} s ({share:.1} % of the time); the first audio came {first_audio_ms} ms after the microphone was asked for."
        ),
        format!("Level: peak {}, average {}.", dbfs(peak), dbfs(rms)),
    ]
}

#[cfg(feature = "dictation")]
use live::run;

#[cfg(feature = "dictation")]
mod live {
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use biorouter::dictation::service::{self, TranscribeRequest};
    use biorouter::dictation::{DictationError, DictionaryRequest};
    use biorouter_dictation_capture::ErrorCode;
    use futures::StreamExt;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::dictation::capture::{CaptureChild, CaptureEvent};
    use crate::dictation::clipboard;
    use crate::dictation::consent::{self, CaptureConsent, PrivateTermsAsk};
    use crate::dictation::controller::{
        self, wipe, Ended, FixedText, LiveDecode, Notice, Recorder, ServiceEngine, StopReason,
    };
    use crate::dictation::environment::{self, CaptureVerdict, Environment};

    /// `BIOROUTER_CLI_DICTATION_DEVICE` names an input device from `biorouter dictation
    /// devices`; unset, the system default.
    pub const DEVICE_ENV: &str = "BIOROUTER_CLI_DICTATION_DEVICE";
    /// The recording's key in the worker's round robin.
    const STREAM: u64 = 1 << 59;
    /// How long the device may take to open.
    const READY_DEADLINE: Duration = Duration::from_secs(10);
    /// How long the child may take to send its last frames after `stop`.
    const DRAIN_DEADLINE: Duration = Duration::from_millis(500);
    /// DD-76: Enter or `e` read within this long after the text is drawn neither uses nor
    /// edits it. A key queued before the text appeared (a held Enter, or a second press of
    /// the one that stopped the recording) is read the moment the review starts, so it
    /// falls inside this window and is dropped; a person who has read the text presses
    /// later. Escape and Ctrl-C still discard at once, since discarding sends nothing.
    pub(super) const REVIEW_GRACE: Duration = Duration::from_millis(300);

    pub async fn run(args: DictateArgs) -> Result<(), Failure> {
        let started = Instant::now();
        let mut timings = DictateTimings::default();

        // Step 2: the managed policy.
        let managed = biorouter::managed::ManagedPolicy::load();
        let policy = managed.dictation();
        for rule in [&policy.enabled, &policy.capture] {
            if let Some(reason) = rule.reason() {
                return Err(Failure::new(Outcome::Policy, reason));
            }
        }
        // `--private-terms` where DD-77 cannot admit it is refused with its own sentence,
        // before the terminal checks would give a more general one (O-8).
        let agent_shell = std::env::var_os(consent::AGENT_SHELL_ENV).is_some_and(|v| !v.is_empty());
        let interactive = crate::commands::needs_terminal::prompt_can_run();
        let mut private_ask = None;
        if args.private_terms {
            let caller_private = !agent_shell
                && interactive
                && crate::session::privacy::caller_capability()
                    .await
                    .is_private();
            let ask = PrivateTermsAsk {
                interactive,
                agent_shell,
                caller_private,
            };
            if !consent::private_terms_admitted(ask) {
                return Err(Failure::new(
                    Outcome::PrivateTermsRefused,
                    messages::MSG_PRIVATE_TERMS_REFUSED,
                ));
            }
            private_ask = Some(ask);
        }
        // `--no-history` where the clipboard cannot honour it: refused before anything
        // starts, rather than a copy that lands in Windows clipboard history anyway.
        if args.copy && args.no_history && !clipboard::Platform::current().honours_no_history() {
            return Err(Failure::new(
                Outcome::NoHistoryRefused,
                messages::MSG_NO_HISTORY_WSL,
            ));
        }
        // Steps 3 and 4: a person at this terminal.
        let consent = CaptureConsent::at_this_terminal().map_err(|verdict| {
            let outcome = match verdict {
                CaptureVerdict::AgentShell => Outcome::AgentShell,
                _ => Outcome::NoTerminal,
            };
            Failure::lines(outcome, verdict.sentences(&Environment::default()))
        })?;
        let env = Environment::sample();
        // Step 5: macOS over SSH.
        if env.mac_over_ssh() {
            return Err(Failure::lines(
                Outcome::NoMicrophone,
                CaptureVerdict::MacOverSsh.sentences(&env),
            ));
        }
        // Step 6: the processor.
        if let Some(refusal) = service::spawn_refusal() {
            return Err(failure_of(refusal));
        }
        let language = match args.language.as_deref() {
            Some(language) => language.trim().to_ascii_lowercase(),
            None => service::configured_language(),
        };
        service::check_language_code(&language).map_err(failure_of)?;

        // The capture child and the worker start together; the probe, the model's
        // resolution and then its load overlap each other and the person's speech.
        let mut child = CaptureChild::spawn()
            .map_err(|_| Failure::new(Outcome::NoMicrophone, messages::MSG_NO_DEVICE))?;
        let svc = service::service();
        svc.spawn_ahead().map_err(failure_of)?;
        let resolve = {
            let model = args.model.clone();
            let language = language.clone();
            tokio::spawn(async move {
                let spec = svc.resolve_for(model.as_deref(), Some(&language)).await;
                (spec, started.elapsed())
            })
        };

        // Step 7: the capture child's probe, before the model.
        let probe = child.probe().await;
        timings.probe_ms = started.elapsed().as_millis() as u64;
        let Some(probe) = probe else {
            return Err(Failure::new(Outcome::NoMicrophone, messages::MSG_NO_DEVICE));
        };
        if let Some(reason) = environment::no_capture_reason(&env, &probe) {
            return Err(Failure::lines(
                Outcome::NoMicrophone,
                CaptureVerdict::NoCapture(reason).sentences(&env),
            ));
        }

        // Step 8: a model.
        let (spec, resolved) = resolve
            .await
            .map_err(|_| Failure::new(Outcome::EngineFailed, messages::MSG_ENGINE_FAILED))?;
        timings.resolve_ms = resolved.as_millis() as u64;
        let spec = spec.map_err(|error| match error {
            DictationError::NoModel { host, account } => {
                let entry =
                    biorouter::dictation::catalog::CATALOG.default_for(service::host_class());
                Failure::new(
                    Outcome::NoModel,
                    messages::msg_no_model_cli(
                        &host,
                        &account,
                        entry.map_or("a speech model", |e| e.id),
                        &entry.map_or_else(String::new, |e| size_words(e.size_bytes)),
                    ),
                )
            }
            other => failure_of(other),
        })?;
        let model_id = service::model_id(&spec);
        // The load (and its warm up) runs while the person speaks.
        let prepared = {
            let spec = spec.clone();
            tokio::spawn(async move {
                let loaded = svc.prepare_with(spec).await;
                (loaded, started.elapsed())
            })
        };

        // The dictionary: public terms, unless the person includes private ones for this
        // one recording (O-8), or `--session` names a private chat (DD-77). Asked before
        // the microphone opens, and prepared while the person speaks. The question reads
        // stdin on a blocking thread, so the model's load goes on while the person decides.
        let included = match private_ask {
            Some(ask) => tokio::task::spawn_blocking(consent::ask_private_terms)
                .await
                .unwrap_or(false)
                .then(|| consent::included_private_terms(ask)),
            None => None,
        };
        let no_dictionary = args.no_dictionary || !policy.dictionary.is_allowed();
        let dictionary = tokio::spawn(prepare_dictionary(
            no_dictionary,
            args.session.clone(),
            included,
            agent_shell,
            interactive,
        ));

        // Step 9: the device.
        let device = std::env::var(DEVICE_ENV).ok().filter(|d| !d.is_empty());
        child
            .start(&consent, device)
            .await
            .map_err(|_| Failure::new(Outcome::MicrophoneBusy, messages::MSG_MIC_BUSY))?;
        let ready = tokio::time::timeout(READY_DEADLINE, async {
            loop {
                match child.next().await {
                    CaptureEvent::Ready(ready) => return Ok(ready),
                    CaptureEvent::Error(code) => return Err(code),
                    CaptureEvent::Exited => return Err(ErrorCode::StartFailed),
                    _ => continue,
                }
            }
        })
        .await
        .unwrap_or(Err(ErrorCode::StartFailed))
        .map_err(device_failure)?;
        timings.device_ready_ms = started.elapsed().as_millis() as u64;
        timings.device_rate = ready.device_rate;

        // The recording. The terminal is in raw mode until the review, so a key is read
        // the moment it is pressed; the guard restores it on every way out.
        let cancel = CancellationToken::new();
        let request = {
            let language = language.clone();
            let cancel = cancel.clone();
            let dictionary = dictionary;
            async move {
                let dictionary = dictionary.await.ok().flatten();
                TranscribeRequest {
                    language,
                    cancel,
                    dictionary,
                    decode: None,
                }
            }
        };
        let (request_tx, request_rx) = tokio::sync::oneshot::channel::<TranscribeRequest>();
        let shared_request = async move {
            let request = request.await;
            let _ = request_tx.send(request.clone());
            request
        };
        let engine: Arc<dyn controller::Engine> = match controller::test_text() {
            Some(text) => Arc::new(FixedText { text }),
            None => Arc::new(ServiceEngine {
                spec: spec.clone(),
                stream: STREAM,
            }),
        };
        let mut live = LiveDecode::new(engine, shared_request);
        let mut recorder = Recorder::new(args.seconds);
        let mut screen = Screen::new();
        let mut keys = crossterm::event::EventStream::new();
        let mut peaks: VecDeque<f32> = VecDeque::from(vec![0.0; 5]);
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        let mut first_audio = None;
        let mut denied = false;
        let mut lost = false;
        let mut capped = false;
        let mut keys_open = true;
        screen.status(messages::LINE_STARTING);
        let stopped_at = loop {
            tokio::select! {
                event = child.next() => match event {
                    CaptureEvent::Audio(mut samples) => {
                        if first_audio.is_none() {
                            first_audio = Some(started.elapsed());
                        }
                        let notices = recorder.push(&samples);
                        wipe(&mut samples);
                        live.poll(recorder.pcm());
                        let mut stop = false;
                        for notice in notices {
                            match notice {
                                Notice::NoAudioYet => screen.note(messages::MSG_NO_AUDIO_YET),
                                Notice::CapWarn => screen.note(messages::MSG_CAP_WARN),
                                Notice::Stop(reason) => {
                                    capped = reason == StopReason::Cap;
                                    stop = true;
                                }
                                Notice::Denied => {
                                    denied = true;
                                    stop = true;
                                }
                            }
                        }
                        if stop {
                            break Instant::now();
                        }
                    }
                    CaptureEvent::Level(level) => {
                        peaks.pop_front();
                        peaks.push_back(level.peak);
                        draw(&mut screen, &recorder, &peaks);
                    }
                    CaptureEvent::Error(_) | CaptureEvent::Exited | CaptureEvent::Stopped(_) => {
                        lost = true;
                        break Instant::now();
                    }
                    _ => {}
                },
                key = keys.next(), if keys_open => match key {
                    None => keys_open = false,
                    Some(event) => match event.ok().and_then(|e| key_of(&e)) {
                        Some(Key::Enter) => break Instant::now(),
                        Some(Key::Escape) | Some(Key::Interrupt) => {
                            screen.clear();
                            return Err(Failure::new(
                                Outcome::Cancelled,
                                messages::MSG_DICTATION_CANCELLED,
                            ));
                        }
                        _ => {}
                    },
                },
                _ = tick.tick() => draw(&mut screen, &recorder, &peaks),
            }
        };
        timings.first_audio_ms = first_audio.map_or(0, |d| d.as_millis() as u64);

        // The stop: the child sends what it holds, then `stopped`, then closes the device.
        if !lost {
            child.stop().await;
            let _ = tokio::time::timeout(DRAIN_DEADLINE, async {
                loop {
                    match child.next().await {
                        CaptureEvent::Audio(mut samples) => {
                            recorder.push(&samples);
                            wipe(&mut samples);
                        }
                        CaptureEvent::Stopped(_)
                        | CaptureEvent::Exited
                        | CaptureEvent::Error(_) => {
                            return;
                        }
                        _ => {}
                    }
                }
            })
            .await;
        }
        timings.stop_to_last_frame_ms = stopped_at.elapsed().as_millis() as u64;
        let closing = tokio::spawn(child.close());
        if lost {
            screen.note(messages::MSG_NO_DEVICE);
        }
        if capped {
            screen.note(messages::MSG_CAP_STOPPED);
        }
        let audio_ms = recorder.duration_ms();
        if denied {
            screen.clear();
            drop(screen);
            return Err(Failure::new(
                Outcome::Silence,
                environment::zero_sentence(&env),
            ));
        }
        let pcm = match recorder.finish() {
            Ok(pcm) => pcm,
            Err(Ended::TooShort) => {
                screen.clear();
                drop(screen);
                return Err(Failure::new(Outcome::TooShort, messages::MSG_TOO_SHORT));
            }
            Err(Ended::Silent) => {
                screen.clear();
                drop(screen);
                return Err(Failure::new(
                    Outcome::Silence,
                    environment::zero_sentence(&env),
                ));
            }
        };

        // The open tail, decoded; Escape or Ctrl-C cancels.
        screen.status(&messages::line_transcribing(audio_ms));
        let decoded = tokio::select! {
            decoded = live.finish(&pcm) => decoded,
            () = wait_for_cancel(&mut keys) => {
                cancel.cancel();
                screen.clear();
                drop(screen);
                return Err(Failure::new(Outcome::Cancelled, messages::MSG_DICTATION_CANCELLED));
            }
        };
        // P-1: the recording is zeroed now, and on the cancel above by its drop.
        drop(pcm);
        let decoded = decoded.map_err(|error| {
            screen.clear();
            match error {
                DictationError::Cancelled => {
                    Failure::new(Outcome::Cancelled, messages::MSG_DICTATION_CANCELLED)
                }
                // `dictate` prints MSG-ENGINE-FAILED-CLI's first sentence and exits 1.
                _ => Failure::new(Outcome::EngineFailed, messages::MSG_ENGINE_FAILED),
            }
        })?;
        let request = request_rx.await.ok();
        let dictionary = request.as_ref().and_then(|r| r.dictionary.clone());
        let raw_text = decoded.raw_text();
        let correcting = Instant::now();
        let (text, changes) = match (&dictionary, raw_text.is_empty()) {
            (Some(dictionary), false) => {
                let corrected = dictionary.correct(&raw_text, &dictionary.prompt_terms());
                (corrected.text, corrected.changes)
            }
            _ => (raw_text.clone(), Vec::new()),
        };
        timings.correct_us = correcting.elapsed().as_micros() as u64;
        timings.stop_to_text_ms = stopped_at.elapsed().as_millis() as u64;
        timings.tail_ms = decoded.tail_ms;
        timings.tail_audio_s = decoded.tail_audio_s;
        timings.segments = decoded.segments;
        timings.segments_while_speaking = decoded.segments_while_speaking;
        timings.encode_ms = decoded.encode_ms;
        timings.decode_ms = decoded.decode_ms;
        if let Ok((Ok(loaded), at)) = prepared.await {
            timings.model_ready_ms = at.as_millis() as u64;
            timings.load_ms = loaded.load_ms;
            timings.warmup_ms = loaded.warmup_ms;
            timings.backend = loaded.backend;
        }
        screen.clear();

        if text.trim().is_empty() {
            drop(screen);
            let _ = closing.await;
            return Err(Failure::new(
                Outcome::NoSpeech,
                if args.copy {
                    messages::MSG_CLIPBOARD_NOTHING
                } else {
                    messages::MSG_NO_SPEECH
                },
            ));
        }
        if args.copy
            && dictionary
                .as_deref()
                .is_some_and(|d| only_prompt_words(&text, &d.prompt_terms()))
        {
            drop(screen);
            let _ = closing.await;
            return Err(Failure::new(
                Outcome::NoSpeech,
                messages::MSG_CLIPBOARD_ECHO,
            ));
        }

        // The review (DD-76): nothing leaves before the person has seen it.
        let text = if args.no_review {
            drop(screen);
            drop(keys);
            text
        } else {
            match review(screen, &text, keys).await {
                Reviewed::Use(text) => text,
                Reviewed::Discard => {
                    let _ = closing.await;
                    return Err(Failure::new(
                        Outcome::Cancelled,
                        messages::MSG_DICTATION_CANCELLED,
                    ));
                }
            }
        };
        let _ = closing.await;

        // The text goes out, and only then does the dictionary learn which terms it used.
        let destination;
        if args.copy {
            destination = "clipboard";
            let (copied, platform) = clipboard::copy(&text, args.no_history, || {
                eprintln!("{}", messages::MSG_CLIPBOARD_WAIT)
            });
            match copied {
                clipboard::Copied::Copied => eprintln!(
                    "{}",
                    messages::msg_clipboard_copied(
                        text.split_whitespace().count(),
                        platform.paste_key()
                    )
                ),
                clipboard::Copied::Unreachable => {
                    record_uses(dictionary.as_deref(), &text);
                    return Err(Failure {
                        outcome: Outcome::ClipboardFallback,
                        lines: vec![messages::MSG_NO_CLIPBOARD.into()],
                        stdout: Some(text),
                    });
                }
            }
        } else {
            destination = "stdout";
        }
        if args.format == DictateFormat::Json {
            let json = DictateJson {
                schema: crate::dictation::SCHEMA,
                text: text.clone(),
                raw_text,
                changes: changes
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
                segments: decoded
                    .pieces
                    .iter()
                    .map(|p| JsonSegment {
                        t0: p.t0_ms as f64 / 1_000.0,
                        t1: p.t1_ms as f64 / 1_000.0,
                        text: p.text.clone(),
                    })
                    .collect(),
                model: model_id,
                language,
                audio_s: audio_ms as f64 / 1_000.0,
                elapsed_ms: timings.stop_to_text_ms,
                bias_scope: dictionary
                    .as_deref()
                    .map_or("public", |d| d.scope().as_str())
                    .into(),
                retried: decoded.retried.map(|reason| JsonRetried {
                    reason: reason.as_str().into(),
                }),
                no_speech: false,
                destination: destination.into(),
                timings,
            };
            let mut out = std::io::stdout().lock();
            let _ = serde_json::to_writer_pretty(&mut out, &json);
            let _ = writeln!(out);
            let _ = out.flush();
        } else if !args.copy {
            let mut out = std::io::stdout().lock();
            let written = writeln!(out, "{text}").and_then(|()| out.flush());
            if let Err(e) = written {
                if e.kind() != std::io::ErrorKind::BrokenPipe {
                    return Err(Failure::new(
                        Outcome::Unwritable,
                        messages::msg_unwritable(None),
                    ));
                }
            }
        }
        record_uses(dictionary.as_deref(), &text);
        Ok(())
    }

    /// `dictation test`: steps 2 to 5 and 7 of the detection order (it needs no model and
    /// no engine), then `seconds` of audio, measured and dropped.
    pub async fn microphone_test(seconds: u32) -> Result<(), Failure> {
        let managed = biorouter::managed::ManagedPolicy::load();
        let policy = managed.dictation();
        for rule in [&policy.enabled, &policy.capture] {
            if let Some(reason) = rule.reason() {
                return Err(Failure::new(Outcome::Policy, reason));
            }
        }
        let consent = CaptureConsent::at_this_terminal().map_err(|verdict| {
            let outcome = match verdict {
                CaptureVerdict::AgentShell => Outcome::AgentShell,
                _ => Outcome::NoTerminal,
            };
            Failure::lines(outcome, verdict.sentences(&Environment::default()))
        })?;
        let env = Environment::sample();
        if env.mac_over_ssh() {
            return Err(Failure::lines(
                Outcome::NoMicrophone,
                CaptureVerdict::MacOverSsh.sentences(&env),
            ));
        }
        let mut child = CaptureChild::spawn()
            .map_err(|_| Failure::new(Outcome::NoMicrophone, messages::MSG_NO_DEVICE))?;
        let Some(probe) = child.probe().await else {
            return Err(Failure::new(Outcome::NoMicrophone, messages::MSG_NO_DEVICE));
        };
        if let Some(reason) = environment::no_capture_reason(&env, &probe) {
            return Err(Failure::lines(
                Outcome::NoMicrophone,
                CaptureVerdict::NoCapture(reason).sentences(&env),
            ));
        }
        let device = std::env::var(DEVICE_ENV).ok().filter(|d| !d.is_empty());
        let asked = Instant::now();
        child
            .start(&consent, device)
            .await
            .map_err(|_| Failure::new(Outcome::MicrophoneBusy, messages::MSG_MIC_BUSY))?;
        let ready = tokio::time::timeout(READY_DEADLINE, async {
            loop {
                match child.next().await {
                    CaptureEvent::Ready(ready) => return Ok(ready),
                    CaptureEvent::Error(code) => return Err(code),
                    CaptureEvent::Exited => return Err(ErrorCode::StartFailed),
                    _ => continue,
                }
            }
        })
        .await
        .unwrap_or(Err(ErrorCode::StartFailed))
        .map_err(device_failure)?;
        let opened = Instant::now();
        let mut recorder = Recorder::new(Some(seconds));
        let mut screen = Screen::new();
        let mut keys = crossterm::event::EventStream::new();
        let mut keys_open = true;
        let mut peaks: VecDeque<f32> = VecDeque::from(vec![0.0; 5]);
        let mut first_audio_ms = None;
        let (mut peak, mut energy, mut samples) = (0f32, 0f64, 0u64);
        let mut denied = false;
        screen.status(messages::LINE_STARTING);
        loop {
            tokio::select! {
                event = child.next() => match event {
                    CaptureEvent::Audio(mut chunk) => {
                        first_audio_ms.get_or_insert(asked.elapsed().as_millis() as u64);
                        for s in &chunk {
                            peak = peak.max(s.abs());
                            energy += f64::from(*s) * f64::from(*s);
                        }
                        samples += chunk.len() as u64;
                        let notices = recorder.push(&chunk);
                        wipe(&mut chunk);
                        if notices.contains(&Notice::Denied) {
                            denied = true;
                            break;
                        }
                        if notices.contains(&Notice::NoAudioYet) {
                            screen.note(messages::MSG_NO_AUDIO_YET);
                        }
                        if notices.iter().any(|n| matches!(n, Notice::Stop(_))) {
                            break;
                        }
                    }
                    CaptureEvent::Level(level) => {
                        peaks.pop_front();
                        peaks.push_back(level.peak);
                        draw(&mut screen, &recorder, &peaks);
                    }
                    CaptureEvent::Error(_) | CaptureEvent::Exited | CaptureEvent::Stopped(_) => {
                        screen.clear();
                        return Err(Failure::new(Outcome::NoMicrophone, messages::MSG_NO_DEVICE));
                    }
                    _ => {}
                },
                key = keys.next(), if keys_open => match key {
                    None => keys_open = false,
                    Some(event) => match event.ok().and_then(|e| key_of(&e)) {
                        Some(Key::Enter) => break,
                        Some(Key::Escape) | Some(Key::Interrupt) => {
                            screen.clear();
                            return Err(Failure::new(
                                Outcome::Cancelled,
                                messages::MSG_DICTATION_CANCELLED,
                            ));
                        }
                        _ => {}
                    },
                },
            }
        }
        let wall = opened.elapsed();
        child.stop().await;
        let _ = child.close().await;
        screen.clear();
        drop(screen);
        if denied {
            return Err(Failure::new(
                Outcome::Silence,
                environment::zero_sentence(&env),
            ));
        }
        let rms = if samples > 0 {
            (energy / samples as f64).sqrt() as f32
        } else {
            0.0
        };
        for line in test_report(&TestRecording {
            device: ready.device,
            backend: ready.backend,
            device_rate: ready.device_rate,
            samples,
            wall,
            first_audio_ms: first_audio_ms.unwrap_or(0),
            peak,
            rms,
        }) {
            println!("{line}");
        }
        Ok(())
    }

    /// The terminal's dictionary for this recording (DD-36, DD-77, O-8): public terms, the
    /// person's private terms when they included them, or a private chat's under the
    /// terminal rule. A store that cannot be read gives no terms.
    async fn prepare_dictionary(
        no_dictionary: bool,
        session: Option<String>,
        included: Option<(
            biorouter::dictation::BiasTarget,
            biorouter::dictation::Standing,
        )>,
        agent_shell: bool,
        interactive: bool,
    ) -> Option<Arc<DictionaryRequest>> {
        use biorouter::dictation::recognize::EligibleEntries;
        use biorouter::dictation::{bias_set, cli_bias, BiasChannels, CliTerminal};
        use biorouter::security::dictionary_store;

        if no_dictionary {
            return None;
        }
        let (target, standing) = match included {
            Some(included) => included,
            None => {
                let mut terminal = CliTerminal {
                    session: None,
                    interactive,
                    caller_private: false,
                    agent_shell,
                };
                if let Some(id) = session.filter(|_| interactive && !agent_shell) {
                    terminal.caller_private = crate::session::privacy::caller_capability()
                        .await
                        .is_private();
                    if terminal.caller_private {
                        terminal.session = biorouter::session::SessionManager::instance()
                            .get_session(&id, false)
                            .await
                            .ok()
                            .map(|s| s.privacy_tier);
                    }
                }
                cli_bias(terminal)
            }
        };
        let enforced = biorouter::privacy::privacy_tiers_enabled();
        let channels = BiasChannels::for_dictation();
        let request = tokio::task::spawn_blocking(move || {
            let eligible = match dictionary_store::open_store().view() {
                Ok(view) => bias_set(&view, &target, standing, enforced),
                Err(_) => EligibleEntries::empty(),
            };
            DictionaryRequest::new(eligible, "", channels)
        })
        .await
        .ok()?;
        let request = Arc::new(request);
        let _ = service::service().install_bias(&request);
        Some(request)
    }

    /// Raises `uses` of the terms `text` holds, once the text is out ("Usage").
    fn record_uses(dictionary: Option<&DictionaryRequest>, text: &str) {
        use biorouter::security::dictionary_store::{self, store};
        if let Some(dictionary) = dictionary {
            let bumps = dictionary.usage_bumps(text, store::schema::now());
            biorouter::dictation::record_usage(
                &dictionary_store::open_store(),
                &bumps,
                Duration::ZERO,
            );
        }
    }

    /// Whether every word of `text` is a word of the prompt's terms: a recording that only
    /// echoed the dictionary (MSG-CLIPBOARD-ECHO).
    pub(super) fn only_prompt_words(text: &str, prompt: &[String]) -> bool {
        let words = |s: &str| -> Vec<String> {
            s.split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .map(str::to_lowercase)
                .collect()
        };
        let known: std::collections::HashSet<String> =
            prompt.iter().flat_map(|t| words(t)).collect();
        let said = words(text);
        !said.is_empty() && said.iter().all(|w| known.contains(w))
    }

    fn device_failure(code: ErrorCode) -> Failure {
        match code {
            ErrorCode::NoDevice | ErrorCode::DeviceMissing | ErrorCode::NoBackend => {
                Failure::new(Outcome::NoMicrophone, messages::MSG_NO_DEVICE)
            }
            ErrorCode::AgentShell => Failure::new(Outcome::AgentShell, messages::MSG_AGENT_SHELL),
            ErrorCode::NotCompiled => {
                Failure::new(Outcome::NotCompiled, biorouter::dictation::MSG_NO_ENGINE)
            }
            _ => Failure::new(Outcome::MicrophoneBusy, messages::MSG_MIC_BUSY),
        }
    }

    /// What a dictation error ends the run with.
    fn failure_of(error: DictationError) -> Failure {
        use DictationError as E;
        let outcome = match &error {
            E::NotCompiled => Outcome::NotCompiled,
            E::UnsupportedCpu { .. } => Outcome::UnsupportedCpu,
            E::Policy(_) => Outcome::Policy,
            E::NoModel { .. } => Outcome::NoModel,
            E::NotInstalled { .. } | E::DetectorNotInstalled { .. } => Outcome::NotInstalled,
            E::UnknownModel(_) => Outcome::UnknownModel,
            E::EnglishOnly { .. } | E::LanguageUnsupported { .. } => Outcome::Language,
            E::Cancelled => Outcome::Cancelled,
            _ => Outcome::EngineFailed,
        };
        let line = match outcome {
            Outcome::EngineFailed => messages::MSG_ENGINE_FAILED.to_string(),
            Outcome::Cancelled => messages::MSG_DICTATION_CANCELLED.to_string(),
            _ => error.to_string(),
        };
        Failure::new(outcome, line)
    }

    /// `574 MB`.
    fn size_words(bytes: u64) -> String {
        if bytes >= 1_000_000 {
            format!("{} MB", bytes / 1_000_000)
        } else {
            format!("{} KB", bytes.div_ceil(1_000))
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Key {
        Enter,
        Escape,
        Interrupt,
        Edit,
        Other,
    }

    fn key_of(event: &crossterm::event::Event) -> Option<Key> {
        use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
        let Event::Key(key) = event else {
            return None;
        };
        if key.kind == KeyEventKind::Release {
            return None;
        }
        Some(match key.code {
            KeyCode::Enter => Key::Enter,
            KeyCode::Esc => Key::Escape,
            KeyCode::Char('c') | KeyCode::Char('d')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                Key::Interrupt
            }
            KeyCode::Char('e') | KeyCode::Char('E') => Key::Edit,
            _ => Key::Other,
        })
    }

    async fn wait_for_cancel(keys: &mut crossterm::event::EventStream) {
        loop {
            match keys.next().await {
                Some(Ok(event)) => {
                    if matches!(key_of(&event), Some(Key::Escape) | Some(Key::Interrupt)) {
                        return;
                    }
                }
                Some(Err(_)) | None => std::future::pending::<()>().await,
            }
        }
    }

    fn draw(screen: &mut Screen, recorder: &Recorder, peaks: &VecDeque<f32>) {
        if !recorder.heard_anything() {
            screen.status(messages::LINE_STARTING);
            return;
        }
        let peaks: Vec<f32> = peaks.iter().copied().collect();
        screen.status(&messages::line_recording(recorder.duration_ms(), &peaks));
    }

    enum Reviewed {
        Use(String),
        Discard,
    }

    /// The review (DD-76): the text on stderr; Enter uses it, `e` edits it on the terminal,
    /// Escape or Ctrl-C discards it. Enter and `e` count only once [`REVIEW_GRACE`] has
    /// passed since the text was drawn, so a key that was already waiting cannot send it.
    async fn review(
        mut screen: Screen,
        text: &str,
        mut keys: crossterm::event::EventStream,
    ) -> Reviewed {
        screen.line(text);
        screen.status(messages::LINE_REVIEW);
        let shown = Instant::now();
        loop {
            let key = match keys.next().await {
                None => None,
                Some(event) => match event.ok().and_then(|e| key_of(&e)) {
                    Some(key) => Some(key),
                    None => continue,
                },
            };
            let key = review_key(key, shown.elapsed());
            match key {
                Some(Key::Enter) => {
                    screen.clear();
                    return Reviewed::Use(text.to_string());
                }
                Some(Key::Escape) | Some(Key::Interrupt) => {
                    screen.clear();
                    return Reviewed::Discard;
                }
                Some(Key::Edit) => {
                    screen.clear();
                    // The line editor reads the terminal itself: nothing else may.
                    drop(keys);
                    drop(screen);
                    let initial = text.to_string();
                    let edited = tokio::task::spawn_blocking(move || edit_line(&initial))
                        .await
                        .ok()
                        .flatten();
                    return match edited {
                        Some(edited) if !edited.trim().is_empty() => Reviewed::Use(edited),
                        _ => Reviewed::Discard,
                    };
                }
                None => {
                    screen.clear();
                    return Reviewed::Discard;
                }
                _ => {}
            }
        }
    }

    /// What a key read `since_shown` after the text was drawn means to the review: Enter
    /// and `e` that came too soon to have been meant for this text count as no key.
    pub(super) fn review_key(key: Option<Key>, since_shown: Duration) -> Option<Key> {
        match key {
            Some(Key::Enter) | Some(Key::Edit) if since_shown < REVIEW_GRACE => Some(Key::Other),
            key => key,
        }
    }

    /// The text on the terminal for editing, with the cursor at its end; `None` when the
    /// person pressed Ctrl-C or Ctrl-D.
    fn edit_line(initial: &str) -> Option<String> {
        let config = rustyline::Config::builder()
            .behavior(rustyline::config::Behavior::PreferTerm)
            .auto_add_history(false)
            .build();
        let mut editor =
            rustyline::Editor::<(), rustyline::history::DefaultHistory>::with_config(config)
                .ok()?;
        #[cfg(unix)]
        let _drawn_on_the_terminal = StdoutOnTerminal::new();
        editor.readline_with_initial("", (initial, "")).ok()
    }

    /// While the line editor runs, stdout points at the terminal on stderr, which
    /// `prompt_can_run()` found to be one. rustyline draws on `/dev/tty` when it can open
    /// it, and otherwise on stdout, which for `"$(biorouter dictate)"` is the pipe the text
    /// is headed for: measured, the edited line arrived there twice, wrapped in the editor's
    /// escape sequences. Nothing else writes to stdout while the person edits. Dropping it
    /// puts stdout back.
    #[cfg(unix)]
    struct StdoutOnTerminal {
        saved: Option<libc::c_int>,
    }

    #[cfg(unix)]
    impl StdoutOnTerminal {
        fn new() -> StdoutOnTerminal {
            use std::io::IsTerminal;
            let _ = std::io::stdout().flush();
            if std::io::stdout().is_terminal() || !std::io::stderr().is_terminal() {
                return StdoutOnTerminal { saved: None };
            }
            // SAFETY: `dup` of this process's own stdout; a failure returns -1.
            let saved = unsafe { libc::dup(libc::STDOUT_FILENO) };
            if saved < 0 {
                return StdoutOnTerminal { saved: None };
            }
            // SAFETY: both are this process's own open standard descriptors.
            if unsafe { libc::dup2(libc::STDERR_FILENO, libc::STDOUT_FILENO) } < 0 {
                // SAFETY: `saved` was opened above and is closed once.
                unsafe { libc::close(saved) };
                return StdoutOnTerminal { saved: None };
            }
            StdoutOnTerminal { saved: Some(saved) }
        }
    }

    #[cfg(unix)]
    impl Drop for StdoutOnTerminal {
        fn drop(&mut self) {
            if let Some(saved) = self.saved.take() {
                let _ = std::io::stdout().flush();
                // SAFETY: `saved` is the stdout `new` duplicated, closed once here.
                unsafe {
                    libc::dup2(saved, libc::STDOUT_FILENO);
                    libc::close(saved);
                }
            }
        }
    }

    /// The terminal while the microphone is open: raw mode, one status line on stderr, and
    /// notes above it. Dropping it restores the terminal.
    struct Screen {
        raw: bool,
        colour: bool,
        drawn: bool,
    }

    impl Screen {
        fn new() -> Screen {
            let raw = crossterm::terminal::enable_raw_mode().is_ok();
            Screen {
                raw,
                colour: std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty()),
                drawn: false,
            }
        }

        fn status(&mut self, line: &str) {
            let mut err = std::io::stderr().lock();
            let shown = if self.colour {
                line.replacen('●', "\x1b[31m●\x1b[0m", 1)
            } else {
                line.to_string()
            };
            let _ = write!(err, "\r\x1b[2K{shown}");
            let _ = err.flush();
            self.drawn = true;
        }

        /// A line above the status line, which is drawn again on the next tick.
        fn note(&mut self, line: &str) {
            let mut err = std::io::stderr().lock();
            let _ = write!(err, "\r\x1b[2K{line}\r\n");
            let _ = err.flush();
            self.drawn = false;
        }

        fn line(&mut self, line: &str) {
            self.note(line);
        }

        fn clear(&mut self) {
            if self.drawn {
                let mut err = std::io::stderr().lock();
                let _ = write!(err, "\r\x1b[2K");
                let _ = err.flush();
                self.drawn = false;
            }
        }
    }

    impl Drop for Screen {
        fn drop(&mut self) {
            self.clear();
            if self.raw {
                let _ = crossterm::terminal::disable_raw_mode();
                self.raw = false;
            }
        }
    }
}

#[cfg(all(test, feature = "dictation"))]
mod tests {
    use super::live::{only_prompt_words, review_key, Key, REVIEW_GRACE};
    use super::*;
    use std::time::Duration;

    /// DD-76: an Enter or `e` read before the grace has passed was waiting before the text
    /// was drawn, so it neither uses nor edits it; Escape and Ctrl-C discard at once.
    #[test]
    fn a_key_that_was_waiting_does_not_review_the_text() {
        let soon = Duration::from_millis(5);
        let later = REVIEW_GRACE + Duration::from_millis(1);
        assert_eq!(review_key(Some(Key::Enter), soon), Some(Key::Other));
        assert_eq!(review_key(Some(Key::Edit), soon), Some(Key::Other));
        assert_eq!(review_key(Some(Key::Escape), soon), Some(Key::Escape));
        assert_eq!(review_key(Some(Key::Interrupt), soon), Some(Key::Interrupt));
        assert_eq!(review_key(None, soon), None);
        assert_eq!(review_key(Some(Key::Enter), later), Some(Key::Enter));
        assert_eq!(review_key(Some(Key::Edit), later), Some(Key::Edit));
        assert!(REVIEW_GRACE >= Duration::from_millis(250));
    }

    /// MSG-CLIPBOARD-ECHO: only words the prompt already holds.
    #[test]
    fn an_echo_is_only_the_prompts_words() {
        let prompt = vec![
            "EPAS1".to_string(),
            "UCSF".to_string(),
            "HIF-2α".to_string(),
        ];
        assert!(only_prompt_words("EPAS1, UCSF.", &prompt));
        assert!(only_prompt_words("hif 2α", &prompt));
        assert!(!only_prompt_words("EPAS1 levels rose", &prompt));
        assert!(!only_prompt_words("", &prompt));
        assert!(!only_prompt_words("EPAS1", &[]));
    }

    #[test]
    fn the_json_error_names_the_outcome() {
        let error = JsonError::new(Outcome::AgentShell, messages::MSG_AGENT_SHELL);
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            serde_json::json!({"schema": 1, "error_code": "agent_shell", "message": messages::MSG_AGENT_SHELL})
        );
        assert_eq!(Outcome::PrivateTermsRefused.exit_code(), 2);
        assert_eq!(Outcome::NoHistoryRefused.exit_code(), 2);
        assert_eq!(Outcome::Silence.exit_code(), 3);
        assert_eq!(Outcome::ClipboardFallback.exit_code(), 5);
        assert_eq!(DictateFormat::parse("json"), DictateFormat::Json);
        assert_eq!(DictateFormat::parse("text"), DictateFormat::Text);
    }
}
