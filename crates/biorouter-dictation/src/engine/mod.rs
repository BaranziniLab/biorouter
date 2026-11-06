//! The safe wrapper over the shim: one whisper.cpp context, its detectors, and decoding.
//!
//! An [`Engine`] lives on the worker's engine thread for as long as the model stays
//! loaded, so a request pays for decoding only. [`Engine::load`] does everything else up
//! front, while the person is still speaking: the model, the separate detector, the
//! integrated detector and, on Metal, one encoder pass that builds the pipelines.
//!
//! Only a worker process uses this module (DD-4): a `GGML_ASSERT` ends in `abort()`.

pub mod booster;
pub mod bpe;
pub mod ffi;

use std::ffi::{c_char, c_void, CStr, CString};
use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicI32;
use std::sync::{Once, OnceLock};
use std::time::Instant;

/// At most 25 s of audio per decode (DD-47).
pub const MAX_TRANSCRIBE_SAMPLES: usize = 25 * 16_000;
/// The sample rate of every buffer the engine takes.
pub const SAMPLE_RATE: u32 = 16_000;

/// The separate detector's thread count, whatever the decode uses. ggml is built without
/// OpenMP (build rule 3), so its CPU backend starts and joins a thread pool for every
/// graph it computes, and Silero computes one graph per 512 samples. Measured with
/// `detector_spans_latency` on an M4 Max at a load average of 12 (2026-11-05), median of
/// five: one thread took 15 ms for 11 s of audio, 83 ms for 60 s and 0.43 s for 300 s;
/// eight threads took 74 ms, 379 ms and 1.82 s.
const DETECTOR_THREADS: i32 = 1;

/// The model reader's buffer. Small on purpose: a read at least this long that finds the
/// buffer empty goes straight from the file into whisper's tensor memory, so the weights
/// are copied once (page cache to tensor), not twice.
const READ_BUFFER_BYTES: usize = 256 * 1024;

/// Everything `Engine::load` needs. Paths are opened by Rust (wide paths on Windows), and
/// both must be absolute: a load moves the process's current directory, so a relative
/// path would name a different file on the next load.
#[derive(Clone, Debug)]
pub struct LoadOptions<'a> {
    pub model_path: &'a Path,
    /// The catalog's size; a file of any other length is refused before it is read.
    pub model_size: u64,
    /// The detector file. Its name must be ASCII: the integrated detector opens it by
    /// name, relative to the process's current directory.
    pub vad_path: &'a Path,
    pub use_gpu: bool,
    pub n_threads: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Metal,
    Cpu,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Metal => "metal",
            Backend::Cpu => "cpu",
        }
    }
}

/// Where the time of `Engine::load` went.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LoadReport {
    /// Opening, checking and loading the model.
    pub model_ms: f64,
    /// The separate detector used by `vad_spans`.
    pub detector_ms: f64,
    /// The integrated detector and, on Metal, one encoder pass.
    pub warmup_ms: f64,
}

/// The DD-47 decoding parameters; [`decode_params`] is the one place they are set.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodeParams {
    pub language: String,
    pub n_threads: u32,
    pub best_of: i32,
    pub temperature: f32,
    pub temperature_inc: f32,
    pub entropy_thold: f32,
    pub logprob_thold: f32,
    pub no_speech_thold: f32,
    pub translate: bool,
    pub no_context: bool,
    pub no_timestamps: bool,
    pub single_segment: bool,
    pub suppress_blank: bool,
    pub suppress_nst: bool,
    pub audio_ctx: i32,
    pub max_tokens: i32,
}

/// The DD-48 voice detection parameters; [`vad_params`] is the one place they are set.
#[derive(Clone, Debug, PartialEq)]
pub struct VadParams {
    /// The integrated detector inside a decode. `vad_spans` always runs its detector.
    pub enabled: bool,
    pub threshold: f32,
    pub min_speech_ms: i32,
    pub min_silence_ms: i32,
    /// `None` is upstream's unlimited default; the parent splits at 25 s.
    pub max_speech_s: Option<f32>,
    pub speech_pad_ms: i32,
    pub samples_overlap_s: f32,
}

/// Greedy decoding with the DD-47 values: temperature 0, `best_of = 5` on fallback only,
/// timestamps on (they keep `small.en` from collapsing) and stripped from the text,
/// `suppress_nst` off (it would block `HLA-B*57:01` and `C++`).
pub fn decode_params(language: &str, n_threads: u32) -> DecodeParams {
    DecodeParams {
        language: language.to_string(),
        n_threads,
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
}

/// Silero with the DD-48 values: threshold 0.50, 250 ms of speech, 100 ms of silence,
/// 30 ms of padding.
pub fn vad_params() -> VadParams {
    VadParams {
        enabled: true,
        threshold: 0.5,
        min_speech_ms: 250,
        min_silence_ms: 100,
        max_speech_s: None,
        speech_pad_ms: 30,
        samples_overlap_s: 0.1,
    }
}

/// `min(8, available_parallelism)`, the worker's thread count before the service halves
/// it beside a serving local model (DD-32).
pub fn default_threads() -> u32 {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8) as u32
}

/// A stretch of speech, in milliseconds from the start of the buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub t0_ms: u64,
    pub t1_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentOut {
    pub t0_ms: u64,
    pub t1_ms: u64,
    /// The segment's text, trimmed. Timestamp tokens never appear in it.
    pub text: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Timings {
    /// The integrated detector plus the mel spectrogram.
    pub vad_ms: f32,
    /// Per encoder pass (one for a segment of 25 s or less).
    pub encode_ms: f32,
    /// The rest of the call: decoding, and any fallback.
    pub decode_ms: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EngineOutput {
    /// Every segment's text, joined and trimmed.
    pub text: String,
    pub segments: Vec<SegmentOut>,
    pub timings: Timings,
}

/// Why the engine refused or failed. `Display` gives a fixed English phrase that never
/// carries a path, a transcript or audio, so a worker may send it to its parent.
#[derive(Debug)]
pub enum EngineError {
    /// The model or detector path is relative; refused before anything is opened.
    RelativePath,
    ModelOpen(std::io::Error),
    ModelSize {
        expected: u64,
        actual: u64,
    },
    /// The model's header is not a Whisper model's; refused before whisper.cpp reads it.
    ModelHeader(&'static str),
    ModelLoad,
    DetectorName,
    DetectorOpen(std::io::Error),
    DetectorHeader(&'static str),
    DetectorLoad,
    WorkingDirectory(std::io::Error),
    /// The integrated detector could not be opened by the warm up.
    DetectorWarmup,
    /// The warm up's encoder pass failed with this whisper.cpp code.
    Warmup(i32),
    TooLong,
    Language,
    Aborted,
    Decode(i32),
    Detector(i32),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let phrase = match self {
            EngineError::RelativePath => "model and voice detector paths must be absolute",
            EngineError::ModelOpen(_) => "model file could not be opened",
            EngineError::ModelSize { .. } => "model file size does not match",
            EngineError::ModelHeader(why) => return write!(f, "model could not be loaded: {why}"),
            EngineError::ModelLoad | EngineError::Warmup(_) => "model could not be loaded",
            EngineError::DetectorName => "voice detector file name must be ASCII",
            EngineError::DetectorHeader(why) => {
                return write!(f, "voice detector could not be loaded: {why}")
            }
            EngineError::DetectorOpen(_)
            | EngineError::DetectorLoad
            | EngineError::DetectorWarmup
            | EngineError::WorkingDirectory(_) => "voice detector could not be loaded",
            EngineError::TooLong => "more than 25 s of audio in one request",
            EngineError::Language => "language code is not valid",
            EngineError::Aborted => "cancelled",
            EngineError::Decode(_) => "decoding failed",
            EngineError::Detector(_) => "voice detection failed",
        };
        f.write_str(phrase)
    }
}

impl std::error::Error for EngineError {}

/// One whisper.cpp context with its two detectors. Created and used on one thread: the
/// raw pointers make it neither `Send` nor `Sync`.
pub struct Engine {
    ctx: *mut ffi::WhisperContext,
    vad: *mut c_void,
    vad_file: CString,
    backend: Backend,
    device: Option<String>,
    n_vocab: i32,
    multilingual: bool,
    report: LoadReport,
    span_buf: Vec<i64>,
}

impl Engine {
    /// Opens the model with `std::fs::File` (wide paths on Windows), checks its length
    /// and header, and feeds it to whisper.cpp through a reader, never through a path
    /// whisper.cpp would open itself (it converts UTF-8 to UTF-16 only under MSVC). Opens
    /// the separate detector the same way. Sets the process's current directory to the
    /// detector's folder, because the integrated detector opens a bare ASCII name relative
    /// to it, and opens that detector now. On Metal, also runs one encoder pass.
    ///
    /// Both paths must be absolute. Making a relative one absolute here would resolve it
    /// against the folder an earlier load moved to, so it is refused instead.
    pub fn load(o: &LoadOptions) -> Result<Engine, EngineError> {
        Engine::load_with_warm_up(o, true)
    }

    /// [`Engine::load`], with the Metal encoder pass only when `warm_up` is true. A load
    /// for one recording (`biorouter transcribe`, PR-12) skips it: on a warm Metal cache
    /// it cost turbo 250 ms and saved the first decode 6 to 14 ms [M]. The detector's
    /// warm up, which opens the integrated detector, always runs.
    pub fn load_with_warm_up(o: &LoadOptions, warm_up: bool) -> Result<Engine, EngineError> {
        if !o.model_path.is_absolute() || !o.vad_path.is_absolute() {
            return Err(EngineError::RelativePath);
        }
        install_logging();

        let vad_dir = o
            .vad_path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let vad_file = o
            .vad_path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| name.is_ascii())
            .and_then(|name| CString::new(name).ok())
            .ok_or(EngineError::DetectorName)?;
        let n_threads = o.n_threads.clamp(1, 64) as i32;

        let started = Instant::now();
        let mut file = File::open(o.model_path).map_err(EngineError::ModelOpen)?;
        let actual = file.metadata().map_err(EngineError::ModelOpen)?.len();
        if actual != o.model_size {
            return Err(EngineError::ModelSize {
                expected: o.model_size,
                actual,
            });
        }
        header::check_model(&mut BufReader::with_capacity(64 * 1024, &mut file), actual)
            .map_err(EngineError::ModelHeader)?;
        file.rewind().map_err(EngineError::ModelOpen)?;
        let reader = Reader::new(file);
        // SAFETY: the reader stays valid for the call, and the shim closes it exactly once.
        let ctx = unsafe { ffi::br_init(&reader.c, o.use_gpu, true) };
        if ctx.is_null() {
            return Err(EngineError::ModelLoad);
        }

        let device = if o.use_gpu { gpu_device() } else { None };
        let backend = if o.use_gpu && device.is_some() {
            Backend::Metal
        } else {
            Backend::Cpu
        };
        let mut engine = Engine {
            ctx,
            vad: std::ptr::null_mut(),
            vad_file,
            backend,
            device,
            // SAFETY: `ctx` is a live context.
            n_vocab: unsafe { ffi::br_n_vocab(ctx) },
            // SAFETY: as above.
            multilingual: unsafe { ffi::br_is_multilingual(ctx) },
            report: LoadReport::default(),
            span_buf: Vec::new(),
        };
        engine.report.model_ms = elapsed_ms(started);

        let started = Instant::now();
        let mut vad = File::open(o.vad_path).map_err(EngineError::DetectorOpen)?;
        let vad_len = vad.metadata().map_err(EngineError::DetectorOpen)?.len();
        header::check_detector(&mut BufReader::with_capacity(4096, &mut vad), vad_len)
            .map_err(EngineError::DetectorHeader)?;
        vad.rewind().map_err(EngineError::DetectorOpen)?;
        let reader = Reader::new(vad);
        // SAFETY: as for the model.
        engine.vad = unsafe { ffi::br_vad_init(&reader.c, DETECTOR_THREADS) };
        if engine.vad.is_null() {
            return Err(EngineError::DetectorLoad);
        }
        engine.report.detector_ms = elapsed_ms(started);

        std::env::set_current_dir(&vad_dir).map_err(EngineError::WorkingDirectory)?;

        let started = Instant::now();
        let vad_on = vad_params();
        let c_vad = vad_on.to_c(&engine.vad_file);
        // On a CPU an encoder pass of a large model costs seconds, more than it saves.
        let encoder = warm_up && engine.backend == Backend::Metal;
        // SAFETY: `ctx` is live and `c_vad` points into `engine.vad_file`.
        let rc = unsafe { ffi::br_warmup(engine.ctx, &c_vad, n_threads, encoder) };
        match rc {
            ffi::BR_OK => {}
            ffi::BR_DETECTOR_FAILED => return Err(EngineError::DetectorWarmup),
            other => return Err(EngineError::Warmup(other)),
        }
        engine.report.warmup_ms = elapsed_ms(started);
        Ok(engine)
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    /// The GPU's description (`Apple M4 Max`), when the engine runs on Metal.
    pub fn device(&self) -> Option<&str> {
        self.device.as_deref()
    }

    pub fn n_vocab(&self) -> i32 {
        self.n_vocab
    }

    pub fn multilingual(&self) -> bool {
        self.multilingual
    }

    pub fn load_report(&self) -> LoadReport {
        self.report
    }

    /// Decodes at most 25 s of 16 kHz mono samples. `abort` set to any non-zero value
    /// stops the decode at the next encoder pass or decoder step.
    pub fn transcribe(
        &mut self,
        pcm: &[f32],
        d: &DecodeParams,
        v: &VadParams,
        abort: &AtomicI32,
    ) -> Result<EngineOutput, EngineError> {
        self.transcribe_with(pcm, d, v, &[], None, abort)
    }

    /// [`Engine::transcribe`] with the dictionary (PR-14): `prompt` is the canonical token
    /// ids passed as `prompt_tokens` (`initial_prompt` stays unset), and `bonus`, when
    /// given, is installed as the decoder bonus for this call alone.
    pub fn transcribe_with(
        &mut self,
        pcm: &[f32],
        d: &DecodeParams,
        v: &VadParams,
        prompt: &[i32],
        bonus: Option<&booster::BonusTable>,
        abort: &AtomicI32,
    ) -> Result<EngineOutput, EngineError> {
        if pcm.len() > MAX_TRANSCRIBE_SAMPLES {
            return Err(EngineError::TooLong);
        }
        let language = CString::new(d.language.as_str()).map_err(|_| EngineError::Language)?;
        let c_decode = d.to_c(&language);
        let c_vad = v.to_c(&self.vad_file);
        let mut timings = ffi::BrTimings::default();
        let (callback, user): (ffi::BrLogitsCb, *mut c_void) = match bonus {
            Some(table) => (
                Some(booster::bonus_callback),
                std::ptr::from_ref(table).cast_mut().cast(),
            ),
            None => (None, std::ptr::null_mut()),
        };
        let (prompt_ptr, n_prompt) = if prompt.is_empty() {
            (std::ptr::null(), 0)
        } else {
            (prompt.as_ptr(), prompt.len() as i32)
        };
        // SAFETY: every pointer is valid for the call; the shim reads `abort` atomically,
        // and the bonus table outlives the call, which is the only time the callback runs.
        let rc = unsafe {
            ffi::br_full(
                self.ctx,
                &c_decode,
                &c_vad,
                pcm.as_ptr(),
                pcm.len() as i32,
                prompt_ptr,
                n_prompt,
                callback,
                user,
                abort.as_ptr().cast_const(),
                &mut timings,
            )
        };
        match rc {
            ffi::BR_OK => {}
            ffi::BR_ABORTED => return Err(EngineError::Aborted),
            ffi::BR_TOO_LONG => return Err(EngineError::TooLong),
            other => return Err(EngineError::Decode(other)),
        }

        // SAFETY: `ctx` is live; segment pointers are valid until the next decode.
        let n = unsafe { ffi::br_n_segments(self.ctx) };
        // Byte level BPE can end a segment inside a multi-byte character, so the joined
        // text is decoded once from every segment's bytes; only each segment on its own
        // is decoded lossily.
        let mut joined: Vec<u8> = Vec::new();
        let mut segments = Vec::with_capacity(n.max(0) as usize);
        for i in 0..n {
            let (mut t0, mut t1) = (0i64, 0i64);
            // SAFETY: as above.
            let raw = unsafe {
                ffi::br_segment_span(self.ctx, i, &mut t0, &mut t1);
                c_bytes(ffi::br_segment_text(self.ctx, i))
            };
            joined.extend_from_slice(raw);
            segments.push(SegmentOut {
                t0_ms: t0.max(0) as u64,
                t1_ms: t1.max(0) as u64,
                text: String::from_utf8_lossy(raw).trim().to_string(),
            });
        }
        Ok(EngineOutput {
            text: String::from_utf8_lossy(&joined).trim().to_string(),
            segments,
            timings: Timings {
                vad_ms: timings.vad_ms,
                encode_ms: timings.encode_ms,
                decode_ms: timings.decode_ms,
            },
        })
    }

    /// The separate detector's speech spans in `pcm` (16 kHz mono), on the CPU.
    pub fn vad_spans(&mut self, pcm: &[f32], v: &VadParams) -> Result<Vec<Span>, EngineError> {
        let n_samples = i32::try_from(pcm.len()).map_err(|_| EngineError::TooLong)?;
        let c_vad = v.to_c(&self.vad_file);
        // A span takes at least 250 ms of speech and 100 ms of silence, so this capacity
        // is never too small for the DD-48 values; the loop covers any others.
        let mut cap = pcm.len() / (SAMPLE_RATE as usize) * 3 + 8;
        loop {
            self.span_buf.resize(cap * 2, 0);
            // SAFETY: the buffer holds `cap` pairs; `vad` is a live detector.
            let n = unsafe {
                ffi::br_vad_spans(
                    self.vad,
                    pcm.as_ptr(),
                    n_samples,
                    &c_vad,
                    self.span_buf.as_mut_ptr(),
                    cap as i32,
                )
            };
            if n < 0 {
                return Err(EngineError::Detector(n));
            }
            let n = n as usize;
            if n > cap {
                cap = n;
                continue;
            }
            return Ok(self
                .span_buf
                .chunks_exact(2)
                .take(n)
                .map(|pair| Span {
                    t0_ms: pair.first().copied().unwrap_or(0).max(0) as u64,
                    t1_ms: pair.get(1).copied().unwrap_or(0).max(0) as u64,
                })
                .collect());
        }
    }

    /// The model's text vocabulary, every id below end of text, for the prompt encoder and
    /// the decoder bonus (PR-14). Read once per load, with one buffer.
    pub fn vocabulary(&self) -> bpe::Vocab {
        let eot = self.token_eot().max(0);
        let mut buf = vec![0u8; 256];
        let mut tokens = Vec::with_capacity(eot as usize);
        for id in 0..eot {
            let mut bytes = Vec::new();
            loop {
                // SAFETY: the buffer holds `buf.len()` bytes; `ctx` is live.
                let len = unsafe {
                    ffi::br_token_bytes(self.ctx, id, buf.as_mut_ptr().cast(), buf.len() as i32)
                };
                if len < 0 {
                    break;
                }
                let len = len as usize;
                if len <= buf.len() {
                    bytes.extend_from_slice(buf.get(..len).unwrap_or_default());
                    break;
                }
                buf.resize(len, 0);
            }
            tokens.push(bytes);
        }
        bpe::Vocab::new(tokens, eot)
    }

    /// `whisper_tokenize`: the greedy longest match whisper.cpp applies to prompt text.
    /// Never used to decode; the tests hold the canonical encoder against it.
    pub fn whisper_tokenize(&self, text: &str) -> Option<Vec<i32>> {
        let text = CString::new(text).ok()?;
        let mut tokens = vec![0i32; text.as_bytes().len() + 16];
        // SAFETY: the buffer holds `tokens.len()` ids; `ctx` is live.
        let n = unsafe {
            ffi::br_tokenize(
                self.ctx,
                text.as_ptr(),
                tokens.as_mut_ptr(),
                tokens.len() as i32,
            )
        };
        let n = usize::try_from(n).ok()?;
        tokens.truncate(n);
        Some(tokens)
    }

    /// The bytes of one vocabulary token.
    pub fn token_bytes(&self, id: i32) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; 64];
        loop {
            // SAFETY: the buffer holds `buf.len()` bytes; `ctx` is live.
            let len = unsafe {
                ffi::br_token_bytes(self.ctx, id, buf.as_mut_ptr().cast(), buf.len() as i32)
            };
            if len < 0 {
                return None;
            }
            let len = len as usize;
            if len <= buf.len() {
                buf.truncate(len);
                return Some(buf);
            }
            buf.resize(len, 0);
        }
    }

    /// The end of text token id.
    pub fn token_eot(&self) -> i32 {
        // SAFETY: `ctx` is live.
        unsafe { ffi::br_token_eot(self.ctx) }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: both pointers came from the shim and are freed once, here.
        unsafe {
            if !self.vad.is_null() {
                ffi::br_vad_free(self.vad);
            }
            ffi::br_free(self.ctx);
        }
    }
}

/// What the parameter builder puts into `whisper_full_params`.
pub use ffi::BrParamsView as ParamsView;

/// The values the shim's one parameter builder produces from `d` and `v`.
pub fn params_snapshot(d: &DecodeParams, v: &VadParams) -> ParamsView {
    let language = CString::new(d.language.as_str()).unwrap_or_default();
    let detector = CString::new("detector.bin").unwrap_or_default();
    let c_decode = d.to_c(&language);
    let c_vad = v.to_c(&detector);
    let mut view = ParamsView::default();
    // SAFETY: all three pointers are valid for the call.
    unsafe { ffi::br_params_snapshot(&c_decode, &c_vad, &mut view) };
    view
}

/// whisper.cpp's system information: the ggml backends and their compiled features.
///
/// Read once per process. `whisper_print_system_info` rebuilds a function static string
/// on every call and returns its buffer, so two threads calling it at once race on that
/// buffer; the compiled features never change while the process runs.
pub fn system_info() -> String {
    static INFO: OnceLock<String> = OnceLock::new();
    INFO.get_or_init(|| {
        // SAFETY: the shim returns a NUL terminated string, and `OnceLock` runs this
        // closure on one thread, which copies the string before anything else calls it.
        unsafe { c_string(ffi::br_system_info()) }
    })
    .clone()
}

/// The version whisper.cpp reports (`1.9.5-dev`).
pub fn whisper_version() -> String {
    // SAFETY: a static string.
    unsafe { c_string(ffi::br_whisper_version()) }
}

/// Installs the DD-13 log filter, once per process.
pub fn install_logging() {
    static INSTALL: Once = Once::new();
    // SAFETY: the shim's callback is a plain function.
    INSTALL.call_once(|| unsafe { ffi::br_log_install() });
}

/// The DD-13 decision for one library log line.
pub fn log_should_forward(level: i32, text: &str) -> bool {
    let Ok(text) = CString::new(text) else {
        return false;
    };
    // SAFETY: a NUL terminated string.
    unsafe { ffi::br_log_should_forward(level, text.as_ptr()) }
}

fn gpu_device() -> Option<String> {
    let mut buf = [0 as c_char; 256];
    // SAFETY: the buffer holds 256 bytes.
    let n = unsafe { ffi::br_gpu_device(buf.as_mut_ptr(), buf.len() as i32) };
    // SAFETY: the shim wrote a NUL terminated string when it returned a length.
    (n > 0).then(|| unsafe { c_string(buf.as_ptr()) })
}

/// # Safety
/// `ptr` is NULL or a NUL terminated string.
unsafe fn c_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    CStr::from_ptr(ptr).to_string_lossy().into_owned()
}

/// # Safety
/// `ptr` is NULL or a NUL terminated string that stays valid for `'a`.
unsafe fn c_bytes<'a>(ptr: *const c_char) -> &'a [u8] {
    if ptr.is_null() {
        return &[];
    }
    CStr::from_ptr(ptr).to_bytes()
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

impl DecodeParams {
    fn to_c(&self, language: &CString) -> ffi::BrDecode {
        ffi::BrDecode {
            language: language.as_ptr(),
            n_threads: self.n_threads.clamp(1, 64) as i32,
            best_of: self.best_of,
            temperature: self.temperature,
            temperature_inc: self.temperature_inc,
            entropy_thold: self.entropy_thold,
            logprob_thold: self.logprob_thold,
            no_speech_thold: self.no_speech_thold,
            translate: self.translate,
            no_context: self.no_context,
            no_timestamps: self.no_timestamps,
            single_segment: self.single_segment,
            suppress_blank: self.suppress_blank,
            suppress_nst: self.suppress_nst,
            audio_ctx: self.audio_ctx,
            max_tokens: self.max_tokens,
        }
    }
}

impl VadParams {
    fn to_c(&self, model_file: &CString) -> ffi::BrVad {
        ffi::BrVad {
            enabled: self.enabled,
            model_file: model_file.as_ptr(),
            threshold: self.threshold,
            min_speech_ms: self.min_speech_ms,
            min_silence_ms: self.min_silence_ms,
            max_speech_s: self.max_speech_s.unwrap_or(f32::MAX),
            speech_pad_ms: self.speech_pad_ms,
            samples_overlap_s: self.samples_overlap_s,
        }
    }
}

/// A file behind the shim's `br_reader`. The shim calls `close` exactly once, which
/// frees the state; if it never ran, `Drop` would leak rather than double free.
struct Reader {
    c: ffi::BrReader,
}

struct ReaderState {
    file: BufReader<File>,
    at_eof: bool,
}

impl Reader {
    fn new(file: File) -> Reader {
        let state = Box::new(ReaderState {
            file: BufReader::with_capacity(READ_BUFFER_BYTES, file),
            at_eof: false,
        });
        Reader {
            c: ffi::BrReader {
                ctx: Box::into_raw(state).cast(),
                read: reader_read,
                eof: reader_eof,
                close: reader_close,
            },
        }
    }
}

unsafe extern "C" fn reader_read(ctx: *mut c_void, out: *mut c_void, n: usize) -> usize {
    if ctx.is_null() || out.is_null() || n == 0 {
        return 0;
    }
    // SAFETY: `ctx` is the `ReaderState` of `Reader::new`; whisper.cpp hands a buffer of
    // `n` writable bytes.
    let state = &mut *ctx.cast::<ReaderState>();
    let buf = std::slice::from_raw_parts_mut(out.cast::<u8>(), n);
    let mut done = 0;
    while done < n {
        let Some(rest) = buf.get_mut(done..) else {
            break;
        };
        match state.file.read(rest) {
            Ok(0) => break,
            Ok(k) => done += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    if done < n {
        state.at_eof = true;
    }
    done
}

unsafe extern "C" fn reader_eof(ctx: *mut c_void) -> bool {
    // SAFETY: as in `reader_read`.
    ctx.is_null() || (*ctx.cast::<ReaderState>()).at_eof
}

unsafe extern "C" fn reader_close(ctx: *mut c_void) {
    if !ctx.is_null() {
        // SAFETY: created by `Box::into_raw` in `Reader::new`; the shim closes once.
        drop(Box::from_raw(ctx.cast::<ReaderState>()));
    }
}

/// A structural check of the model and detector files before whisper.cpp reads them.
///
/// whisper.cpp sizes its buffers from the header: the hyperparameters decide the tensor
/// allocation, and a tensor record's name length decides a buffer before the name is
/// checked. A corrupt header could therefore ask for gigabytes before anything refuses
/// it (the CVE-2026-110810 class). The parent verifies a catalog model's SHA-256, so this
/// is the second line: only Whisper's own architectures pass, and every tensor record
/// must lie inside the file.
mod header {
    use std::io::{BufReader, Read, Seek};

    const GGML_FILE_MAGIC: u32 = 0x6767_6d6c;
    const STATES: [i32; 5] = [384, 512, 768, 1024, 1280];
    const HEADS: [i32; 5] = [6, 8, 12, 16, 20];
    const AUDIO_LAYERS: [i32; 5] = [4, 6, 12, 24, 32];
    const TEXT_LAYERS: [i32; 6] = [2, 4, 6, 12, 24, 32];
    const MELS: [i32; 2] = [80, 128];
    const MAX_TOKEN_BYTES: u32 = 1024;
    const MAX_TENSOR_NAME: i32 = 256;

    struct Cursor<'a, R: Read + Seek> {
        r: &'a mut BufReader<R>,
        pos: u64,
        len: u64,
    }

    impl<R: Read + Seek> Cursor<'_, R> {
        fn bytes4(&mut self) -> Result<[u8; 4], &'static str> {
            let mut b = [0u8; 4];
            self.r
                .read_exact(&mut b)
                .map_err(|_| "the file ends early")?;
            self.pos += 4;
            Ok(b)
        }

        fn i32(&mut self) -> Result<i32, &'static str> {
            self.bytes4().map(i32::from_le_bytes)
        }

        fn u32(&mut self) -> Result<u32, &'static str> {
            self.bytes4().map(u32::from_le_bytes)
        }

        fn skip(&mut self, n: u64) -> Result<(), &'static str> {
            let end = self.pos.checked_add(n).ok_or("the file ends early")?;
            if end > self.len {
                return Err("the file ends early");
            }
            let n = i64::try_from(n).map_err(|_| "the file ends early")?;
            self.r.seek_relative(n).map_err(|_| "the file ends early")?;
            self.pos = end;
            Ok(())
        }

        fn remaining(&self) -> u64 {
            self.len.saturating_sub(self.pos)
        }
    }

    /// (block size, bytes per block) of the tensor types Whisper models use.
    fn type_size(ttype: i32) -> Option<(u64, u64)> {
        match ttype {
            0 => Some((1, 4)),   // F32
            1 => Some((1, 2)),   // F16
            2 => Some((32, 18)), // Q4_0
            3 => Some((32, 20)), // Q4_1
            6 => Some((32, 22)), // Q5_0
            7 => Some((32, 24)), // Q5_1
            8 => Some((32, 34)), // Q8_0
            30 => Some((1, 2)),  // BF16
            _ => None,
        }
    }

    pub(super) fn check_model<R: Read + Seek>(
        r: &mut BufReader<R>,
        len: u64,
    ) -> Result<(), &'static str> {
        let mut c = Cursor { r, pos: 0, len };
        if c.u32()? != GGML_FILE_MAGIC {
            return Err("not a ggml model file");
        }
        let mut hp = [0i32; 11];
        for value in hp.iter_mut() {
            *value = c.i32()?;
        }
        let [n_vocab, n_audio_ctx, n_audio_state, n_audio_head, n_audio_layer, n_text_ctx, n_text_state, n_text_head, n_text_layer, n_mels, ftype] =
            hp;
        if !(51_800..=52_000).contains(&n_vocab) {
            return Err("the vocabulary is not a Whisper vocabulary");
        }
        if n_audio_ctx != 1500 || n_text_ctx != 448 {
            return Err("the context sizes are not Whisper's");
        }
        if !STATES.contains(&n_audio_state) || n_text_state != n_audio_state {
            return Err("the state width is not a Whisper width");
        }
        if !HEADS.contains(&n_audio_head) || !HEADS.contains(&n_text_head) {
            return Err("the head count is not a Whisper head count");
        }
        if !AUDIO_LAYERS.contains(&n_audio_layer) || !TEXT_LAYERS.contains(&n_text_layer) {
            return Err("the layer count is not a Whisper layer count");
        }
        if !MELS.contains(&n_mels) {
            return Err("the mel band count is not Whisper's");
        }
        if !(0..100_000).contains(&ftype) {
            return Err("the weight type is not valid");
        }

        let n_mel = c.i32()?;
        let n_fft = c.i32()?;
        if n_mel != n_mels || !(1..=1024).contains(&n_fft) {
            return Err("the mel filters do not match the header");
        }
        c.skip(n_mel as u64 * n_fft as u64 * 4)?;

        let tokens = c.i32()?;
        if !(0..=n_vocab).contains(&tokens) {
            return Err("the vocabulary section does not match the header");
        }
        for _ in 0..tokens {
            let len = c.u32()?;
            if len > MAX_TOKEN_BYTES {
                return Err("a vocabulary entry is too long");
            }
            c.skip(u64::from(len))?;
        }

        // whisper.cpp reads 12 bytes per record and stops at the end of the file.
        while c.remaining() >= 12 {
            let n_dims = c.i32()?;
            let name_len = c.i32()?;
            let ttype = c.i32()?;
            if !(0..=4).contains(&n_dims) {
                return Err("a tensor has an invalid rank");
            }
            if !(1..=MAX_TENSOR_NAME).contains(&name_len) {
                return Err("a tensor name is too long");
            }
            let (block, bytes) = type_size(ttype).ok_or("a tensor has an unknown type")?;
            let mut elements: u64 = 1;
            for _ in 0..n_dims {
                let ne = c.i32()?;
                if ne < 1 {
                    return Err("a tensor has an invalid shape");
                }
                elements = elements
                    .checked_mul(ne as u64)
                    .filter(|e| *e <= i32::MAX as u64)
                    .ok_or("a tensor is too large")?;
            }
            if !elements.is_multiple_of(block) {
                return Err("a tensor does not fill its blocks");
            }
            c.skip(name_len as u64)?;
            c.skip(elements / block * bytes)?;
        }
        Ok(())
    }

    pub(super) fn check_detector<R: Read + Seek>(
        r: &mut BufReader<R>,
        len: u64,
    ) -> Result<(), &'static str> {
        let mut c = Cursor { r, pos: 0, len };
        if c.u32()? != GGML_FILE_MAGIC {
            return Err("not a ggml detector file");
        }
        let name_len = c.i32()?;
        if !(1..=64).contains(&name_len) {
            return Err("the detector header is not valid");
        }
        c.skip(name_len as u64)?;
        Ok(())
    }

    #[cfg(test)]
    pub(super) mod build {
        //! Writers of model headers, for the tests.

        pub struct Model {
            pub hparams: [i32; 11],
            pub n_fft: i32,
            pub tokens: Vec<Vec<u8>>,
            /// (n_dims, name_len, ttype, dims, data bytes)
            pub tensors: Vec<(i32, i32, i32, Vec<i32>, usize)>,
        }

        impl Default for Model {
            fn default() -> Model {
                Model {
                    // base.en
                    hparams: [51864, 1500, 512, 8, 6, 448, 512, 8, 6, 80, 1],
                    n_fft: 201,
                    tokens: vec![b"!".to_vec(), b"hello".to_vec()],
                    tensors: Vec::new(),
                }
            }
        }

        /// The builder writes at most this many bytes of any one section: a corrupt
        /// header claims sizes no test should allocate, and the check refuses it before
        /// reading that far.
        const SECTION_CAP: usize = 1 << 20;

        impl Model {
            pub fn bytes(&self) -> Vec<u8> {
                let mut out = Vec::new();
                let mut push = |v: i32| out.extend_from_slice(&v.to_le_bytes());
                push(0x6767_6d6c);
                for v in self.hparams {
                    push(v);
                }
                push(self.hparams[9]);
                push(self.n_fft);
                let filters = (self.hparams[9].max(0) as usize)
                    .saturating_mul(self.n_fft.max(0) as usize)
                    .saturating_mul(4)
                    .min(SECTION_CAP);
                out.resize(out.len() + filters, 0);
                out.extend_from_slice(&(self.tokens.len() as i32).to_le_bytes());
                for token in &self.tokens {
                    out.extend_from_slice(&(token.len() as u32).to_le_bytes());
                    out.extend_from_slice(token);
                }
                for (n_dims, name_len, ttype, dims, data) in &self.tensors {
                    for v in [*n_dims, *name_len, *ttype] {
                        out.extend_from_slice(&v.to_le_bytes());
                    }
                    for d in dims {
                        out.extend_from_slice(&d.to_le_bytes());
                    }
                    let name = ((*name_len).max(0) as usize).min(SECTION_CAP);
                    out.resize(out.len() + name + (*data).min(SECTION_CAP), b'x');
                }
                out
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::{Mutex, MutexGuard};

    /// Tests that load an engine hold this: each load sets the process's current
    /// directory, and one Metal context at a time keeps a test run predictable.
    pub(crate) fn engine_lock() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_info::fixtures;
    use std::io::Cursor;

    #[test]
    fn the_rust_structs_match_the_c_layout() {
        // SAFETY: a pure function.
        let size = |which| unsafe { ffi::br_sizeof(which) };
        assert_eq!(size(0), std::mem::size_of::<ffi::BrReader>());
        assert_eq!(size(1), std::mem::size_of::<ffi::BrDecode>());
        assert_eq!(size(2), std::mem::size_of::<ffi::BrVad>());
        assert_eq!(size(3), std::mem::size_of::<ffi::BrTimings>());
        assert_eq!(size(4), std::mem::size_of::<ffi::BrParamsView>());
    }

    /// DD-13 and P-12: only short warnings and errors reach stderr, and the parameter
    /// builder never asks whisper.cpp to print the transcript.
    #[test]
    fn log_filter_drops_long_lines() {
        let long = format!("{}\n", "w".repeat(299));
        assert_eq!(long.len(), 300);
        assert!(!log_should_forward(ffi::GGML_LOG_LEVEL_WARN, &long));
        assert!(!log_should_forward(ffi::GGML_LOG_LEVEL_ERROR, &long));
        let short = format!("{}\n", "w".repeat(79));
        assert_eq!(short.len(), 80);
        assert!(log_should_forward(ffi::GGML_LOG_LEVEL_WARN, &short));
        assert!(log_should_forward(ffi::GGML_LOG_LEVEL_ERROR, &short));
        assert!(log_should_forward(
            ffi::GGML_LOG_LEVEL_WARN,
            &"x".repeat(200)
        ));
        assert!(!log_should_forward(
            ffi::GGML_LOG_LEVEL_WARN,
            &"x".repeat(201)
        ));
        for level in [
            ffi::GGML_LOG_LEVEL_INFO,
            ffi::GGML_LOG_LEVEL_DEBUG,
            ffi::GGML_LOG_LEVEL_CONT,
        ] {
            assert!(!log_should_forward(level, &short), "level {level}");
        }

        let view = params_snapshot(&decode_params("en", 6), &vad_params());
        assert_eq!(
            (
                view.print_special,
                view.print_progress,
                view.print_realtime,
                view.print_timestamps
            ),
            (false, false, false, false)
        );
        assert_eq!(view.strategy, 0, "greedy");
        assert_eq!(view.best_of, 5);
        assert!(view.no_context);
        assert!(!view.no_timestamps, "timestamps stay on (DD-47)");
        assert!(!view.single_segment);
        assert!(view.vad);
        assert!((view.temperature_inc - 0.2).abs() < f32::EPSILON);
        assert!(!view.suppress_nst);
        assert_eq!(view.n_threads, 6);
    }

    #[test]
    fn the_fixture_models_pass_the_header_check() {
        for relative in [fixtures::BASE_EN, fixtures::TINY] {
            let bytes = std::fs::read(fixtures::path(relative)).expect("fixture");
            let len = bytes.len() as u64;
            header::check_model(&mut BufReader::new(Cursor::new(bytes)), len)
                .unwrap_or_else(|why| panic!("{relative}: {why}"));
        }
        let bytes = std::fs::read(fixtures::path(fixtures::SILERO)).expect("fixture");
        let len = bytes.len() as u64;
        header::check_detector(&mut BufReader::new(Cursor::new(bytes)), len).expect("detector");
    }

    #[test]
    fn corrupt_headers_are_refused() {
        let check = |model: header::build::Model| {
            let bytes = model.bytes();
            let len = bytes.len() as u64;
            header::check_model(&mut BufReader::new(Cursor::new(bytes)), len)
        };
        let good = || header::build::Model::default();
        assert_eq!(check(good()), Ok(()));

        let mut m = good();
        m.hparams[0] = i32::MAX;
        assert!(check(m).is_err(), "huge vocabulary");
        let mut m = good();
        m.hparams[2] = 1 << 20;
        m.hparams[6] = 1 << 20;
        assert!(check(m).is_err(), "huge state");
        let mut m = good();
        m.hparams[4] = 4096;
        assert!(check(m).is_err(), "huge layer count");
        let mut m = good();
        m.n_fft = 1 << 30;
        assert!(check(m).is_err(), "huge mel filter");
        let mut m = good();
        m.tokens.push(vec![b'a'; 4096]);
        assert!(check(m).is_err(), "huge token");
        let mut m = good();
        m.tensors.push((2, i32::MAX, 1, vec![512, 512], 0));
        assert!(check(m).is_err(), "huge tensor name");
        let mut m = good();
        m.tensors.push((2, 8, 1, vec![i32::MAX, i32::MAX], 0));
        assert!(check(m).is_err(), "huge tensor");
        let mut m = good();
        m.tensors.push((2, 8, 1, vec![512, 512], 100));
        assert!(check(m).is_err(), "tensor data past the end");
        let mut m = good();
        m.tensors.push((2, 8, 1, vec![512, 4], 512 * 4 * 2));
        assert_eq!(check(m), Ok(()), "a well formed tensor");
        let mut m = good();
        m.tensors.push((2, 8, 99, vec![512, 4], 0));
        assert!(check(m).is_err(), "unknown tensor type");

        let mut bytes = good().bytes();
        bytes[0] = b'x';
        let len = bytes.len() as u64;
        assert!(header::check_model(&mut BufReader::new(Cursor::new(bytes)), len).is_err());
    }

    #[test]
    fn a_detector_name_outside_ascii_is_refused() {
        let _lock = test_support::engine_lock();
        let model = fixtures::path(fixtures::BASE_EN);
        let size = std::fs::metadata(&model).expect("fixture").len();
        let vad = std::env::temp_dir().join("d\u{e9}tecteur.bin");
        let result = Engine::load(&LoadOptions {
            model_path: &model,
            model_size: size,
            vad_path: &vad,
            use_gpu: false,
            n_threads: 2,
        });
        assert!(matches!(result, Err(EngineError::DetectorName)));
    }

    /// A load moves the current directory, so a relative path would name another file on
    /// the next load (an unload then load, or a model switch). Both paths are refused
    /// before anything is opened, after a first load has already moved the directory.
    #[test]
    fn a_relative_path_is_refused_on_every_load() {
        let _lock = test_support::engine_lock();
        let model = fixtures::path(fixtures::BASE_EN);
        let vad = fixtures::path(fixtures::SILERO);
        let size = std::fs::metadata(&model).expect("fixture").len();
        let load = |model_path: &Path, vad_path: &Path| {
            Engine::load(&LoadOptions {
                model_path,
                model_size: size,
                vad_path,
                use_gpu: false,
                n_threads: 2,
            })
        };
        drop(load(&model, &vad).expect("the absolute fixture paths load"));
        let relative_model = Path::new("models").join("for-tests-ggml-base.en.bin");
        let relative_vad = Path::new("for-tests-silero-v6.2.0-ggml.bin");
        assert!(matches!(
            load(&relative_model, &vad),
            Err(EngineError::RelativePath)
        ));
        assert!(matches!(
            load(&model, relative_vad),
            Err(EngineError::RelativePath)
        ));
        assert_eq!(
            load(relative_vad, relative_vad)
                .err()
                .map(|e| e.to_string())
                .as_deref(),
            Some("model and voice detector paths must be absolute")
        );
        drop(load(&model, &vad).expect("a second load from absolute paths"));
    }

    #[test]
    fn a_wrong_size_is_refused_before_loading() {
        let _lock = test_support::engine_lock();
        let model = fixtures::path(fixtures::BASE_EN);
        let size = std::fs::metadata(&model).expect("fixture").len();
        let vad = fixtures::path(fixtures::SILERO);
        let result = Engine::load(&LoadOptions {
            model_path: &model,
            model_size: size + 1,
            vad_path: &vad,
            use_gpu: false,
            n_threads: 2,
        });
        assert!(matches!(result, Err(EngineError::ModelSize { .. })));
        assert_eq!(
            result.err().map(|e| e.to_string()).as_deref(),
            Some("model file size does not match")
        );
    }

    #[test]
    fn the_fixture_engine_loads_on_the_cpu() {
        let _lock = test_support::engine_lock();
        let model = fixtures::path(fixtures::BASE_EN);
        let size = std::fs::metadata(&model).expect("fixture").len();
        let vad = fixtures::path(fixtures::SILERO);
        let engine = Engine::load(&LoadOptions {
            model_path: &model,
            model_size: size,
            vad_path: &vad,
            use_gpu: false,
            n_threads: 2,
        })
        .expect("the fixture loads");
        assert_eq!(engine.backend(), Backend::Cpu);
        assert_eq!(engine.device(), None);
        assert!(!engine.multilingual());
        assert_eq!(engine.n_vocab(), 51864);
        assert!(engine.token_eot() > 0);
        assert!(engine.token_bytes(engine.token_eot()).is_some());
        assert_eq!(engine.token_bytes(-1), None);
        assert!(whisper_version().starts_with("1.9"));
    }
}
