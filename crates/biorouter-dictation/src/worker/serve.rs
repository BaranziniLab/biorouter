//! The engine role: the frame protocol on stdin and stdout (`engine-and-models.md`,
//! "Inside the worker").
//!
//! Two threads. The reader thread parses stdin frames: `cancel` sets the abort flag of
//! the request it names, `shutdown` and end of file set it for whatever runs, and every
//! other message goes to the engine thread with its payload moved, not copied. The
//! engine thread owns the one [`Engine`] and writes every stdout frame. One request runs
//! at a time; the model stays loaded between requests, so a warm request pays for the
//! decode alone.
//!
//! Stdout carries frames only. On Unix the frames go to a duplicate of descriptor 1 and
//! descriptor 1 itself is pointed at stderr, so a stray `printf` from C or C++ code can
//! never corrupt a frame.

use std::collections::VecDeque;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use super::exit;
use crate::audio;
use crate::build_info;
use crate::cpu::{self, HardwareClass};
use crate::engine::booster::BonusTable;
use crate::engine::bpe::Vocab;
use crate::engine::{self, Engine, LoadOptions};
use crate::protocol::{
    self, BiasReady, ErrorCode, ErrorMsg, FrameError, FromWorker, Hello, Load, Loaded, SegmentText,
    Segments, SetBias, Timings, ToWorker, Transcribe, TranscribeResult, VadOptions, VadSegments,
    BIAS_TABLES, PROTOCOL_VERSION,
};
use crate::recognize::prompt;

/// How many recent `cancel` ids the reader remembers for requests not yet started.
const REMEMBERED_CANCELS: usize = 64;

/// The engine role's entry point: this process's stdin, stdout and CPU.
pub fn serve_stdio() -> i32 {
    no_crash_dumps();
    let mut out = FrameWriter::new(frame_output());
    let report = cpu::host_report();
    // The worker trusts only its own CPUID view: under Rosetta the parent advertises
    // AVX2 through the environment, and this is where that is confirmed.
    let refused = report.class == HardwareClass::No || !report.missing.is_empty();
    if !refused {
        engine::install_logging();
    }
    let hello = hello(refused);
    if out.send(&FromWorker::Hello(hello)).is_err() {
        return exit::OK;
    }
    if refused {
        let sentence = cpu::sentence(report).unwrap_or_else(|| {
            cpu::MSG_CPU.replace("{missing}", &cpu::display_list(&report.missing))
        });
        eprintln!("{sentence}");
        return exit::UNSUPPORTED_CPU;
    }
    run(io::stdin(), out)
}

/// P-1: this process holds the audio it decodes (and from PR-14 prompt and bias terms),
/// and it is the one expected to abort (DD-4), so a crash must not write its memory to
/// disk. On Unix the core file limit goes to zero, soft and hard, which systemd-coredump
/// honors too; on Linux the process also stops being dumpable, which keeps a debugger of
/// the same account from attaching to read it. On Windows a crash does not invoke
/// Windows Error Reporting, and a report that is still made (a fail fast abort) leaves
/// out the heap. Runs before the first frame is read.
fn no_crash_dumps() {
    #[cfg(unix)]
    {
        let none = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: lowers a limit of this process; the struct outlives the call.
        unsafe {
            libc::setrlimit(libc::RLIMIT_CORE, &none);
        }
    }
    #[cfg(target_os = "linux")]
    {
        // SAFETY: sets one flag of this process.
        unsafe {
            libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Diagnostics::Debug::{
            SetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX,
        };
        use windows_sys::Win32::System::ErrorReporting::{
            WerSetFlags, WER_FAULT_REPORTING_FLAG_NOHEAP,
        };
        // SAFETY: both set flags of this process and take no pointers.
        unsafe {
            SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
            WerSetFlags(WER_FAULT_REPORTING_FLAG_NOHEAP);
        }
    }
}

/// The worker's first frame. `system_info` calls into ggml, so it is empty when the
/// gate refused.
fn hello(refused: bool) -> Hello {
    let backends = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        vec!["metal".to_string(), "cpu".to_string()]
    } else {
        vec!["cpu".to_string()]
    };
    Hello {
        protocol: PROTOCOL_VERSION,
        whisper: build_info::WHISPER_COMMIT.to_string(),
        ggml: build_info::GGML_VERSION.to_string(),
        backends,
        cpu: cpu::detect().into(),
        isa: build_info::ISA.to_string(),
        system_info: if refused {
            String::new()
        } else {
            engine::system_info()
        },
    }
}

/// The frame loop after `hello`: a reader thread on `input`, the engine on this thread.
/// Returns the exit code: 0 after `shutdown` or end of file, 65 after a bad frame.
pub fn run<R: Read + Send + 'static>(input: R, mut out: FrameWriter) -> i32 {
    let shared = Arc::new(Shared::default());
    out.quiet = Arc::clone(&shared.quiet);
    let (tx, rx) = mpsc::channel();
    let reader_shared = Arc::clone(&shared);
    let reader = std::thread::Builder::new()
        .name("br-dictation-reader".into())
        .spawn(move || read_loop(input, &reader_shared, &tx));
    if reader.is_err() {
        return exit::PANICKED;
    }
    let mut worker = EngineThread {
        shared,
        out,
        engine: None,
        loaded: None,
        pcm: Vec::new(),
        vocab: None,
        tables: VecDeque::new(),
        last_prompt: None,
    };
    let code = worker.serve(&rx);
    // Frees the model, the detectors and the Metal device before the process exits.
    drop(worker.engine.take());
    code
}

/// What the reader thread and the engine thread share.
#[derive(Default)]
struct Shared {
    /// Polled by whisper.cpp between encoder passes and decoder steps.
    abort: AtomicI32,
    /// The id of the request the engine is decoding; 0 when none.
    running: AtomicU64,
    /// `shutdown` or end of file: nothing more is started.
    stop: AtomicBool,
    /// A protocol error: nothing more is written.
    quiet: Arc<AtomicBool>,
    /// `cancel` ids that may name a request not yet started.
    cancelled: Mutex<VecDeque<u64>>,
}

impl Shared {
    fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.abort.store(1, Ordering::SeqCst);
    }

    fn cancel(&self, id: u64) {
        if let Ok(mut cancelled) = self.cancelled.lock() {
            if cancelled.len() == REMEMBERED_CANCELS {
                cancelled.pop_front();
            }
            cancelled.push_back(id);
        }
        // Remembered first, then checked: a request either sees its id in the list when
        // it starts, or is already running here and is aborted.
        if self.running.load(Ordering::SeqCst) == id {
            self.abort.store(1, Ordering::SeqCst);
        }
    }

    /// Marks `id` as running. False when it was cancelled, or the worker is stopping.
    fn begin(&self, id: u64) -> bool {
        self.abort.store(0, Ordering::SeqCst);
        self.running.store(id, Ordering::SeqCst);
        let cancelled = self
            .cancelled
            .lock()
            .map(|list| list.contains(&id))
            .unwrap_or(false);
        !cancelled && !self.stop.load(Ordering::SeqCst)
    }

    fn end(&self) {
        self.running.store(0, Ordering::SeqCst);
    }
}

/// Messages the reader thread hands to the engine thread.
enum Incoming {
    Load(Load),
    SetBias(SetBias),
    Transcribe(Transcribe, Vec<u8>),
    Vad(VadSegments, Vec<u8>),
    /// A type this worker does not know.
    Unsupported(Option<u64>),
    /// A known type whose fields could not be read.
    Unreadable(Option<u64>),
    Shutdown,
    Protocol,
}

fn read_loop<R: Read>(mut input: R, shared: &Shared, tx: &mpsc::Sender<Incoming>) {
    loop {
        let frame = match protocol::read_frame_blocking(&mut input) {
            Ok(frame) => frame,
            Err(FrameError::Closed) => {
                // The parent closed stdin or died: abort and stop. Dropping `tx` ends
                // the engine loop once it has drained.
                shared.stop();
                return;
            }
            Err(_) => return protocol_error(shared, tx),
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&frame.json) else {
            return protocol_error(shared, tx);
        };
        let id = value.get("id").and_then(serde_json::Value::as_u64);
        let incoming = match serde_json::from_value::<ToWorker>(value) {
            Err(_) => Incoming::Unreadable(id),
            Ok(ToWorker::Cancel { id }) => {
                shared.cancel(id);
                continue;
            }
            Ok(ToWorker::Shutdown) => {
                shared.stop();
                let _ = tx.send(Incoming::Shutdown);
                return;
            }
            Ok(ToWorker::Load(load)) => Incoming::Load(load),
            Ok(ToWorker::Transcribe(t)) => Incoming::Transcribe(t, frame.payload),
            Ok(ToWorker::VadSegments(v)) => Incoming::Vad(v, frame.payload),
            Ok(ToWorker::SetBias(set)) => {
                // Only a decoder bonus table reads the English word list, and the bonus is
                // off by default (DD-44), so the list is decoded only when a table is on
                // its way: here, on the reader thread, as soon as its frame arrives.
                warm_word_list();
                Incoming::SetBias(set)
            }
            Ok(ToWorker::Unknown) => Incoming::Unsupported(id),
        };
        if tx.send(incoming).is_err() {
            return;
        }
    }
}

/// A frame that cannot be read leaves the stream at an unknown offset: write nothing
/// more and exit 65.
fn protocol_error(shared: &Shared, tx: &mpsc::Sender<Incoming>) {
    shared.quiet.store(true, Ordering::SeqCst);
    shared.stop();
    let _ = tx.send(Incoming::Protocol);
}

struct EngineThread {
    shared: Arc<Shared>,
    out: FrameWriter,
    engine: Option<Engine>,
    /// The `load` the engine answers, and its answer, for an idempotent second `load`.
    loaded: Option<(Load, Loaded)>,
    /// The request's samples, reused: the capacity stays, the samples are zeroed.
    pcm: Vec<f32>,
    /// The loaded model's text vocabulary, read once per load, for the prompt encoder
    /// and the decoder bonus.
    vocab: Option<Arc<Vocab>>,
    /// Built decoder bonus tables by (scope, revision), least recently used first; at
    /// most [`BIAS_TABLES`]. Cleared by every load, because a table is built over one
    /// model's vocabulary.
    tables: VecDeque<(String, u64, BonusTable)>,
    /// The last prompt encoded: its terms, its budget, its ids and how many terms they
    /// hold. Cleared with the vocabulary.
    last_prompt: Option<(Vec<String>, usize, Vec<i32>, usize)>,
}

impl EngineThread {
    fn serve(&mut self, rx: &mpsc::Receiver<Incoming>) -> i32 {
        loop {
            let Ok(incoming) = rx.recv() else {
                return exit::OK;
            };
            // After `shutdown` or end of file nothing more is started or answered.
            let stopping = self.shared.stop.load(Ordering::SeqCst);
            let reply = match incoming {
                Incoming::Shutdown => return exit::OK,
                Incoming::Protocol => return exit::PROTOCOL,
                Incoming::Transcribe(_, mut payload) | Incoming::Vad(_, mut payload)
                    if stopping =>
                {
                    clear_audio(&mut payload, &mut self.pcm);
                    continue;
                }
                _ if stopping => continue,
                Incoming::Load(load) => self.load(load),
                Incoming::SetBias(set) => self.set_bias(&set),
                Incoming::Transcribe(request, mut payload) => {
                    let reply = self.transcribe(&request, &payload);
                    clear_audio(&mut payload, &mut self.pcm);
                    reply
                }
                Incoming::Vad(request, mut payload) => {
                    let reply = self.vad_segments(&request, &payload);
                    clear_audio(&mut payload, &mut self.pcm);
                    reply
                }
                Incoming::Unsupported(id) => error(id, ErrorCode::Internal, "unsupported message"),
                Incoming::Unreadable(id) => {
                    error(id, ErrorCode::Internal, "message could not be read")
                }
            };
            if self.out.send(&reply).is_err() {
                // The parent is gone; nobody reads what follows.
                return exit::OK;
            }
        }
    }

    fn load(&mut self, load: Load) -> FromWorker {
        if let (Some(_), Some((current, answer))) = (&self.engine, &self.loaded) {
            // The thread count is per request, and a model already loaded needs no warm
            // up; any other difference is another model.
            let same = Load {
                n_threads: current.n_threads,
                warm_up: current.warm_up,
                ..load.clone()
            };
            if *current == same {
                return FromWorker::Loaded(answer.clone());
            }
        }
        // The previous model's memory is returned before the next one is read, and its
        // vocabulary and bonus tables with it.
        self.engine = None;
        self.loaded = None;
        self.vocab = None;
        self.tables.clear();
        self.last_prompt = None;
        let bare_ascii = !load.vad_file.is_empty()
            && load.vad_file.is_ascii()
            && !load.vad_file.contains(['/', '\\'])
            && load.vad_file != ".."
            && load.vad_file != ".";
        if !bare_ascii {
            return error(
                None,
                ErrorCode::ModelLoadFailed,
                "voice detector file name must be ASCII",
            );
        }
        // The English word list is not decoded here: only a decoder bonus table reads it,
        // and with the bonus off by default (DD-44) a load that decoded it spent about
        // 20 ms of a debug build's CPU [M] on a list nothing read. The reader thread starts
        // it when a `set_bias` frame arrives (`warm_word_list`).
        let model_path = PathBuf::from(&load.model_path);
        let vad_path = Path::new(&load.vad_dir).join(&load.vad_file);
        let options = LoadOptions {
            model_path: &model_path,
            model_size: load.model_size,
            vad_path: &vad_path,
            use_gpu: load.use_gpu,
            n_threads: load.n_threads,
        };
        let engine = match Engine::load_with_warm_up(&options, load.warm_up) {
            Ok(engine) => engine,
            Err(e) => return error(None, ErrorCode::ModelLoadFailed, &e.to_string()),
        };
        // Read here, while the person is still speaking, so neither the first prompt nor
        // the first bonus table pays for it.
        self.vocab = Some(Arc::new(engine.vocabulary()));
        let report = engine.load_report();
        let answer = Loaded {
            load_ms: (report.model_ms + report.detector_ms).round() as u64,
            warmup_ms: report.warmup_ms.round() as u64,
            backend: engine.backend().as_str().to_string(),
            device: engine.device().map(str::to_string),
            rss_bytes: peak_rss_bytes(),
            n_vocab: engine.n_vocab(),
            multilingual: engine.multilingual(),
        };
        self.engine = Some(engine);
        self.loaded = Some((load, answer.clone()));
        FromWorker::Loaded(answer)
    }

    /// Builds a decoder bonus table over the loaded model's vocabulary and keeps it under
    /// its (scope, revision), the least recently used of [`BIAS_TABLES`] going first.
    /// Answered `bias_ready`; with no model loaded nothing is kept and `nodes` is 0, and a
    /// `transcribe` naming the table then decodes without the bonus.
    fn set_bias(&mut self, set: &SetBias) -> FromWorker {
        let started = std::time::Instant::now();
        let ready = |nodes: usize, started: std::time::Instant| {
            FromWorker::BiasReady(BiasReady {
                revision: set.revision,
                scope: set.scope.clone(),
                nodes: nodes as u64,
                build_ms: started.elapsed().as_millis() as u64,
            })
        };
        let Some(vocab) = self.vocab.as_ref().filter(|_| self.engine.is_some()) else {
            return ready(0, started);
        };
        let table = BonusTable::build(set, Arc::clone(vocab));
        let nodes = table.nodes();
        self.tables
            .retain(|(scope, revision, _)| !(*scope == set.scope && *revision == set.revision));
        self.tables
            .push_back((set.scope.clone(), set.revision, table));
        while self.tables.len() > BIAS_TABLES {
            self.tables.pop_front();
        }
        ready(nodes, started)
    }

    fn transcribe(&mut self, request: &Transcribe, payload: &[u8]) -> FromWorker {
        let id = Some(request.id);
        let Some(engine) = self.engine.as_mut() else {
            return error(id, ErrorCode::Internal, "no model loaded");
        };
        if request.decode.strategy != "greedy" {
            return error(id, ErrorCode::Internal, "unsupported message");
        }
        if let Err(e) = audio::validate_s16le(payload, protocol::MAX_TRANSCRIBE_SAMPLES) {
            return error(id, ErrorCode::BadAudio, &e.to_string());
        }
        // The prompt: canonical BPE over this model's vocabulary, cut from the head to the
        // budget (DD-45).
        // A stream's segments all carry the same prompt, so the last one encoded is kept and
        // the tail segment decoded at stop does not encode it again.
        let budget = request
            .prompt_budget_tokens
            .min(protocol::PROMPT_BUDGET_TOKENS) as usize;
        let cached = self
            .last_prompt
            .as_ref()
            .filter(|(terms, b, _, _)| *terms == request.prompt_terms && *b == budget)
            .map(|(_, _, ids, used)| (ids.clone(), *used));
        let (prompt_ids, prompt_used) = match (&self.vocab, cached) {
            (_, Some(hit)) => hit,
            (Some(vocab), None) if !request.prompt_terms.is_empty() => {
                let fitted = prompt::fit(&request.prompt_terms, budget, |text| vocab.encode(text));
                self.last_prompt = Some((
                    request.prompt_terms.clone(),
                    budget,
                    fitted.0.clone(),
                    fitted.1,
                ));
                fitted
            }
            _ => (Vec::new(), 0),
        };
        // The bonus table the request names, touched as the most recently used; one the
        // worker no longer holds means a decode without the bonus.
        let table_at = request.bias.as_ref().and_then(|bias| {
            self.tables
                .iter()
                .position(|(scope, revision, _)| *scope == bias.scope && *revision == bias.revision)
        });
        if let Some(at) = table_at {
            if let Some(entry) = self.tables.remove(at) {
                self.tables.push_back(entry);
            }
        }
        let bonus = table_at.and_then(|_| self.tables.back()).map(|(_, _, t)| t);
        audio::s16le_to_f32(payload, &mut self.pcm);
        if !self.shared.begin(request.id) {
            self.shared.end();
            return error(id, ErrorCode::Cancelled, "cancelled");
        }
        let decoded = engine.transcribe_with(
            &self.pcm,
            &decode_params(request),
            &vad_params(&request.vad),
            &prompt_ids,
            bonus,
            &self.shared.abort,
        );
        self.shared.end();
        match decoded {
            Ok(output) => FromWorker::Result(TranscribeResult {
                id: request.id,
                text: output.text,
                segments: output
                    .segments
                    .into_iter()
                    .map(|s| SegmentText {
                        t0_ms: s.t0_ms,
                        t1_ms: s.t1_ms,
                        text: s.text,
                    })
                    .collect(),
                prompt_terms_used: prompt_used as u32,
                timings: Timings {
                    vad_ms: output.timings.vad_ms,
                    encode_ms: output.timings.encode_ms,
                    decode_ms: output.timings.decode_ms,
                },
            }),
            Err(engine::EngineError::Aborted) => error(id, ErrorCode::Cancelled, "cancelled"),
            Err(engine::EngineError::TooLong) => {
                error(id, ErrorCode::BadAudio, "too much audio in one request")
            }
            Err(e) => error(id, ErrorCode::Internal, &e.to_string()),
        }
    }

    fn vad_segments(&mut self, request: &VadSegments, payload: &[u8]) -> FromWorker {
        let id = Some(request.id);
        let Some(engine) = self.engine.as_mut() else {
            return error(id, ErrorCode::Internal, "no model loaded");
        };
        if let Err(e) = audio::validate_s16le(payload, protocol::MAX_VAD_SAMPLES) {
            return error(id, ErrorCode::BadAudio, &e.to_string());
        }
        audio::s16le_to_f32(payload, &mut self.pcm);
        if !self.shared.begin(request.id) {
            self.shared.end();
            return error(id, ErrorCode::Cancelled, "cancelled");
        }
        let spans = engine.vad_spans(&self.pcm, &vad_params(&request.vad));
        self.shared.end();
        match spans {
            Ok(spans) => FromWorker::Segments(Segments {
                id: request.id,
                spans: spans
                    .into_iter()
                    .map(|s| audio::Span {
                        t0_ms: s.t0_ms,
                        t1_ms: s.t1_ms,
                    })
                    .collect(),
            }),
            Err(e) => error(id, ErrorCode::Internal, &e.to_string()),
        }
    }
}

/// P-1 hygiene: no request's audio outlives it, in either buffer.
fn clear_audio(payload: &mut Vec<u8>, pcm: &mut Vec<f32>) {
    payload.fill(0);
    payload.clear();
    pcm.fill(0.0);
    pcm.clear();
}

/// Starts decoding the English word list on a thread of its own, once per process. A thread
/// that cannot be started leaves the decode to the first table that reads the list.
fn warm_word_list() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("word-list".into())
            .spawn(crate::recognize::words::warm);
    });
}

fn error(id: Option<u64>, code: ErrorCode, message: &str) -> FromWorker {
    FromWorker::Error(ErrorMsg {
        id,
        code,
        message: message.to_string(),
    })
}

/// The DD-47 parameters as the parent sent them, with no prompt token limit.
fn decode_params(request: &Transcribe) -> engine::DecodeParams {
    let d = &request.decode;
    engine::DecodeParams {
        language: request.language.clone(),
        n_threads: d.n_threads,
        best_of: d.best_of,
        temperature: d.temperature,
        temperature_inc: d.temperature_inc,
        entropy_thold: d.entropy_thold,
        logprob_thold: d.logprob_thold,
        no_speech_thold: d.no_speech_thold,
        translate: d.translate,
        no_context: d.no_context,
        no_timestamps: d.no_timestamps,
        single_segment: d.single_segment,
        suppress_blank: d.suppress_blank,
        suppress_nst: d.suppress_nst,
        audio_ctx: d.audio_ctx,
        max_tokens: 0,
    }
}

fn vad_params(v: &VadOptions) -> engine::VadParams {
    engine::VadParams {
        enabled: v.enabled,
        threshold: v.threshold,
        min_speech_ms: v.min_speech_ms,
        min_silence_ms: v.min_silence_ms,
        max_speech_s: v.max_speech_s,
        speech_pad_ms: v.speech_pad_ms,
        samples_overlap_s: v.samples_overlap_s,
    }
}

/// Writes frames to the parent, one flush per frame.
pub struct FrameWriter {
    out: BufWriter<Box<dyn Write + Send>>,
    quiet: Arc<AtomicBool>,
}

impl FrameWriter {
    pub fn new(out: Box<dyn Write + Send>) -> FrameWriter {
        FrameWriter {
            out: BufWriter::with_capacity(64 * 1024, out),
            quiet: Arc::default(),
        }
    }

    fn send(&mut self, message: &FromWorker) -> io::Result<()> {
        if self.quiet.load(Ordering::SeqCst) {
            return Ok(());
        }
        protocol::write_frame_blocking(&mut self.out, &protocol::to_json(message), &[])
    }
}

/// Where frames go. On Unix a duplicate of descriptor 1; descriptor 1 itself becomes a
/// copy of stderr, so C and C++ code that prints cannot reach the frame stream.
/// Windows keeps stdout (pipes are binary there) and relies on the shim never asking
/// whisper.cpp to print (DD-13).
#[cfg(unix)]
fn frame_output() -> Box<dyn Write + Send> {
    use std::os::fd::FromRawFd;
    // SAFETY: plain descriptor calls; the duplicate is owned by the returned File.
    unsafe {
        let fd = libc::dup(1);
        if fd < 0 {
            return Box::new(io::stdout());
        }
        if libc::dup2(2, 1) < 0 {
            libc::close(fd);
            return Box::new(io::stdout());
        }
        Box::new(std::fs::File::from_raw_fd(fd))
    }
}

#[cfg(not(unix))]
fn frame_output() -> Box<dyn Write + Send> {
    Box::new(io::stdout())
}

/// This process's peak resident size.
fn peak_rss_bytes() -> Option<u64> {
    #[cfg(unix)]
    {
        // SAFETY: `getrusage` fills the struct it is handed; zero is a valid start.
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        // SAFETY: as above.
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
            return None;
        }
        let max = u64::try_from(usage.ru_maxrss).ok()?;
        // Bytes on macOS, KiB everywhere else.
        Some(if cfg!(target_vendor = "apple") {
            max
        } else {
            max.saturating_mul(1024)
        })
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::ProcessStatus::{
            K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        // SAFETY: the counters struct is plain data; zero is a valid start.
        let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        counters.cb = size;
        // SAFETY: the pseudo handle of this process and a struct of the size passed.
        let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) };
        (ok != 0).then_some(counters.PeakWorkingSetSize as u64)
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_info::fixtures;
    use crate::engine::test_support::engine_lock;
    use crate::protocol::{read_frame_blocking, to_json, write_frame_blocking, DecodeOptions};
    use std::io::{PipeReader, PipeWriter};
    use std::time::{Duration, Instant};

    /// A worker on two pipes, as the supervisor sees one.
    struct PipeWorker {
        to_worker: Option<PipeWriter>,
        from_worker: PipeReader,
        thread: Option<std::thread::JoinHandle<i32>>,
    }

    impl PipeWorker {
        fn start() -> PipeWorker {
            let (input, to_worker) = std::io::pipe().expect("a pipe");
            let (from_worker, output) = std::io::pipe().expect("a pipe");
            let thread = std::thread::spawn(move || run(input, FrameWriter::new(Box::new(output))));
            PipeWorker {
                to_worker: Some(to_worker),
                from_worker,
                thread: Some(thread),
            }
        }

        fn send(&mut self, message: &ToWorker, payload: &[u8]) {
            let pipe = self.to_worker.as_mut().expect("stdin is open");
            write_frame_blocking(pipe, &to_json(message), payload).expect("write a frame");
        }

        fn send_raw(&mut self, bytes: &[u8]) {
            let pipe = self.to_worker.as_mut().expect("stdin is open");
            pipe.write_all(bytes).expect("write");
        }

        fn recv(&mut self) -> FromWorker {
            let frame = read_frame_blocking(&mut self.from_worker).expect("a frame");
            assert!(frame.payload.is_empty(), "the worker sends no payloads");
            serde_json::from_slice(&frame.json).expect("a message")
        }

        fn close_stdin(&mut self) {
            self.to_worker = None;
        }

        fn exit_code(&mut self) -> i32 {
            self.thread
                .take()
                .expect("running")
                .join()
                .expect("no panic")
        }
    }

    fn fixture_load() -> Load {
        let model = fixtures::path(fixtures::BASE_EN);
        let vad = fixtures::path(fixtures::SILERO);
        Load {
            model_path: model.to_str().expect("UTF-8").to_string(),
            model_sha256: fixtures::BASE_EN_SHA256.to_string(),
            model_size: std::fs::metadata(&model).expect("fixture").len(),
            vad_dir: vad
                .parent()
                .expect("dir")
                .to_str()
                .expect("UTF-8")
                .to_string(),
            vad_file: vad
                .file_name()
                .and_then(|n| n.to_str())
                .expect("name")
                .to_string(),
            use_gpu: false,
            n_threads: 2,
            warm_up: true,
        }
    }

    fn jfk_payload() -> Vec<u8> {
        let bytes = std::fs::read(fixtures::path(fixtures::JFK)).expect("jfk.wav");
        let wav = audio::parse_wav_s16(&bytes).expect("jfk.wav is 16 bit PCM");
        assert_eq!((wav.sample_rate, wav.channels), (16_000, 1));
        audio::s16le_bytes(&wav.samples).into_owned()
    }

    fn transcribe(id: u64) -> ToWorker {
        ToWorker::Transcribe(Transcribe {
            id,
            bias: None,
            prompt_terms: vec![],
            prompt_budget_tokens: protocol::PROMPT_BUDGET_TOKENS,
            language: "en".into(),
            decode: DecodeOptions::dd47(2),
            vad: VadOptions::dd48(),
        })
    }

    fn error_of(message: FromWorker) -> (Option<u64>, ErrorCode, String) {
        match message {
            FromWorker::Error(e) => (e.id, e.code, e.message),
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[test]
    fn the_wire_parameters_equal_the_engines() {
        let request = transcribe(1);
        let ToWorker::Transcribe(request) = request else {
            unreachable!()
        };
        assert_eq!(decode_params(&request), engine::decode_params("en", 2));
        assert_eq!(vad_params(&VadOptions::dd48()), engine::vad_params());
    }

    /// PR-12's parameter snapshot (DD-47, DD-48): the frame every file and stream sends,
    /// through the worker's conversion, every field of the engine's `DecodeParams` and
    /// `VadParams`; and through the shim's one parameter builder, the fields its
    /// `br_params_view` reads back: the strategy, `best_of`, `no_context`,
    /// `no_timestamps`, `single_segment`, the four print flags, the detector switch,
    /// `temperature_inc`, `suppress_nst` and the thread count. The builder's other fields
    /// (the thresholds, `suppress_blank`, `translate`, the temperature, the language, the
    /// prompt fields and the context's `flash_attn`) are checked on `DecodeParams` only;
    /// reading them back too needs `br_params_view` to grow. Greedy with `best_of = 5`,
    /// timestamps on, `temperature_inc 0.2`, `suppress_nst` off, the detector on with its
    /// four values, and at most 25 s of audio on the wire and in the engine. The
    /// supervisor refuses 25.5 s before it queues anything
    /// (`audio_over_the_caps_is_refused_before_it_is_queued`) and the worker answers a
    /// longer payload `bad_audio` (`a_worker_answers_every_request_in_order`).
    #[test]
    fn the_decode_parameters_snapshot() {
        let ToWorker::Transcribe(request) = transcribe(1) else {
            unreachable!()
        };
        let decode = decode_params(&request);
        assert_eq!(
            decode,
            engine::DecodeParams {
                language: "en".into(),
                n_threads: 2,
                best_of: 5,
                temperature: 0.0,
                temperature_inc: 0.2,
                entropy_thold: 2.4,
                logprob_thold: -1.0,
                no_speech_thold: 0.6,
                translate: false,
                no_context: true,
                no_timestamps: false,
                single_segment: false,
                suppress_blank: true,
                suppress_nst: false,
                audio_ctx: 0,
                max_tokens: 0,
            }
        );
        let vad = vad_params(&request.vad);
        assert_eq!(
            vad,
            engine::VadParams {
                enabled: true,
                threshold: 0.5,
                min_speech_ms: 250,
                min_silence_ms: 100,
                max_speech_s: None,
                speech_pad_ms: 30,
                samples_overlap_s: 0.1,
            }
        );
        assert_eq!(
            engine::params_snapshot(&decode, &vad),
            engine::ParamsView {
                strategy: 0,
                best_of: 5,
                no_context: true,
                no_timestamps: false,
                single_segment: false,
                print_special: false,
                print_progress: false,
                print_realtime: false,
                print_timestamps: false,
                vad: true,
                temperature_inc: 0.2,
                suppress_nst: false,
                n_threads: 2,
            }
        );
        // A multilingual model is told the configured language, never `auto` unless set.
        let mut german = request.clone();
        german.language = "de".into();
        assert_eq!(decode_params(&german).language, "de");
        assert_eq!(protocol::MAX_TRANSCRIBE_SAMPLES, 25 * 16_000);
        assert_eq!(
            protocol::MAX_TRANSCRIBE_SAMPLES,
            engine::MAX_TRANSCRIBE_SAMPLES
        );
        // The parent keeps 0.3 s of audio before the first and after the last stretch of
        // speech of every segment it plans (DD-48; `audio::plan_final_segments`).
        assert_eq!(audio::EDGE_PAD_MS, 300);
        assert!(request.prompt_terms.is_empty() && request.bias.is_none());
    }

    /// P-1: after the setup the engine role runs first, this process writes no core file
    /// and, on Linux, is not dumpable.
    #[cfg(unix)]
    #[test]
    fn a_worker_writes_no_core_file() {
        no_crash_dumps();
        let mut limit = libc::rlimit {
            rlim_cur: 1,
            rlim_max: 1,
        };
        // SAFETY: reads a limit of this process into the struct it is handed.
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut limit) }, 0);
        assert_eq!((limit.rlim_cur, limit.rlim_max), (0, 0));
        #[cfg(target_os = "linux")]
        {
            // SAFETY: reads one flag of this process.
            let dumpable = unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) };
            assert_eq!(dumpable, 0);
        }
    }

    #[test]
    fn hello_reports_this_build() {
        let hello = hello(false);
        assert_eq!(hello.protocol, 1);
        assert_eq!(hello.whisper, "4da27c41");
        assert_eq!(hello.isa, build_info::ISA);
        assert!(hello.system_info.contains("CPU :"), "{}", hello.system_info);
        assert_eq!(hello.backends.last().map(String::as_str), Some("cpu"));
        assert!(cpu::missing_features(&hello.cpu.into()).is_empty());
        assert_eq!(super::hello(true).system_info, "");
    }

    /// Load, the idempotent second load, the detector, a decode, and the error answers,
    /// over real pipes with the fixture model.
    #[test]
    fn a_worker_answers_every_request_in_order() {
        let _lock = engine_lock();
        let mut worker = PipeWorker::start();

        worker.send(&transcribe(1), &[0; 64]);
        assert_eq!(
            error_of(worker.recv()),
            (Some(1), ErrorCode::Internal, "no model loaded".to_string())
        );

        worker.send(&ToWorker::Load(fixture_load()), &[]);
        let FromWorker::Loaded(loaded) = worker.recv() else {
            panic!("expected loaded")
        };
        assert_eq!(loaded.backend, "cpu");
        assert_eq!(loaded.n_vocab, 51_864);
        assert!(!loaded.multilingual);
        assert!(loaded.rss_bytes.is_some_and(|rss| rss > 1 << 20));
        // The same model again is answered from memory, figures and all.
        worker.send(&ToWorker::Load(fixture_load()), &[]);
        assert_eq!(worker.recv(), FromWorker::Loaded(loaded.clone()));

        let jfk = jfk_payload();
        worker.send(
            &ToWorker::VadSegments(VadSegments {
                id: 2,
                vad: VadOptions::dd48(),
            }),
            &jfk,
        );
        let FromWorker::Segments(segments) = worker.recv() else {
            panic!("expected segments")
        };
        assert_eq!(segments.id, 2);
        assert!(!segments.spans.is_empty());
        assert!(segments
            .spans
            .iter()
            .all(|s| s.t0_ms < s.t1_ms && s.t1_ms <= 11_100));

        worker.send(&transcribe(3), &jfk);
        let FromWorker::Result(result) = worker.recv() else {
            panic!("expected a result")
        };
        assert_eq!(result.id, 3);
        // The fixture model has no tensors, so it decodes to nothing [M].
        assert_eq!(result.text, "");
        assert_eq!(result.prompt_terms_used, 0);

        worker.send(&transcribe(4), &[0; 3]);
        assert_eq!(error_of(worker.recv()).1, ErrorCode::BadAudio);
        worker.send(&transcribe(5), &vec![0; 2 * 400_001]);
        assert_eq!(error_of(worker.recv()).1, ErrorCode::BadAudio);
        // PR-14: a bonus table is built and answered, and a request with a prompt and the
        // table is decoded, the whole prompt fitting the budget.
        worker.send(
            &ToWorker::SetBias(protocol::SetBias {
                revision: 1,
                scope: "public".into(),
                entries: vec![protocol::BiasEntry {
                    written: "EPAS1".into(),
                    bias: "auto".into(),
                }],
                params: protocol::BonusParams::default(),
            }),
            &[],
        );
        let FromWorker::BiasReady(ready) = worker.recv() else {
            panic!("expected bias_ready")
        };
        assert_eq!((ready.revision, ready.scope.as_str()), (1, "public"));
        assert!(ready.nodes > 1);
        let mut prompted = transcribe(6);
        if let ToWorker::Transcribe(t) = &mut prompted {
            t.prompt_terms = vec!["Biorouter".into(), "EPAS1".into()];
            t.bias = Some(protocol::BiasRef {
                revision: 1,
                scope: "public".into(),
            });
        }
        worker.send(&prompted, &jfk);
        let FromWorker::Result(result) = worker.recv() else {
            panic!("expected a result")
        };
        assert_eq!((result.id, result.prompt_terms_used), (6, 2));
        // A table the worker does not hold decodes without the bonus.
        let mut unknown = transcribe(7);
        if let ToWorker::Transcribe(t) = &mut unknown {
            t.bias = Some(protocol::BiasRef {
                revision: 9,
                scope: "all".into(),
            });
        }
        worker.send(&unknown, &jfk);
        assert!(matches!(worker.recv(), FromWorker::Result(r) if r.id == 7));
        worker.send_raw(&protocol::frame_head(br#"{"type":"warm_up","id":9}"#, 0).expect("head"));
        assert_eq!(
            error_of(worker.recv()),
            (
                Some(9),
                ErrorCode::Internal,
                "unsupported message".to_string()
            )
        );
        worker
            .send_raw(&protocol::frame_head(br#"{"type":"transcribe","id":10}"#, 0).expect("head"));
        assert_eq!(
            error_of(worker.recv()),
            (
                Some(10),
                ErrorCode::Internal,
                "message could not be read".to_string()
            )
        );

        // A request cancelled before it starts is answered `cancelled`.
        worker.send(&ToWorker::Cancel { id: 11 }, &[]);
        worker.send(&transcribe(11), &jfk);
        assert_eq!(error_of(worker.recv()).1, ErrorCode::Cancelled);
        // And the next one runs.
        worker.send(&transcribe(12), &jfk);
        assert!(matches!(worker.recv(), FromWorker::Result(r) if r.id == 12));

        worker.send(&ToWorker::Shutdown, &[]);
        assert_eq!(worker.exit_code(), exit::OK);
    }

    #[test]
    fn a_failed_load_is_answered_and_the_worker_stays() {
        let _lock = engine_lock();
        let mut worker = PipeWorker::start();
        let mut wrong_size = fixture_load();
        wrong_size.model_size += 1;
        worker.send(&ToWorker::Load(wrong_size), &[]);
        assert_eq!(
            error_of(worker.recv()),
            (
                None,
                ErrorCode::ModelLoadFailed,
                "model file size does not match".to_string()
            )
        );
        let mut nested = fixture_load();
        nested.vad_file = format!("../{}", nested.vad_file);
        worker.send(&ToWorker::Load(nested), &[]);
        assert_eq!(error_of(worker.recv()).1, ErrorCode::ModelLoadFailed);
        worker.send(&ToWorker::Load(fixture_load()), &[]);
        assert!(matches!(worker.recv(), FromWorker::Loaded(_)));
        worker.close_stdin();
        assert_eq!(worker.exit_code(), exit::OK);
    }

    #[test]
    fn end_of_file_ends_a_loaded_worker_at_once() {
        let _lock = engine_lock();
        let mut worker = PipeWorker::start();
        worker.send(&ToWorker::Load(fixture_load()), &[]);
        assert!(matches!(worker.recv(), FromWorker::Loaded(_)));
        let started = Instant::now();
        worker.close_stdin();
        assert_eq!(worker.exit_code(), exit::OK);
        assert!(started.elapsed() < Duration::from_secs(1));
        // Nothing more was written.
        assert!(matches!(
            read_frame_blocking(&mut worker.from_worker),
            Err(FrameError::Closed)
        ));
    }

    #[test]
    fn a_bad_frame_ends_the_worker_with_65_and_nothing_more_is_written() {
        let mut worker = PipeWorker::start();
        let mut oversized = Vec::new();
        oversized.extend_from_slice(&u32::MAX.to_le_bytes());
        worker.send_raw(&oversized);
        assert_eq!(worker.exit_code(), exit::PROTOCOL);
        assert!(matches!(
            read_frame_blocking(&mut worker.from_worker),
            Err(FrameError::Closed)
        ));

        let mut worker = PipeWorker::start();
        worker.send_raw(&protocol::frame_head(b"not json", 0).expect("head"));
        assert_eq!(worker.exit_code(), exit::PROTOCOL);
    }
}
