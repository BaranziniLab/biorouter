//! Hand written declarations of `shim/br_whisper.h` (build rule 8: no bindgen).
//!
//! Every `#[repr(C)]` struct mirrors the C struct field for field, in the same order and
//! with the same types; `br_sizeof` lets a test hold the two layouts together.

use std::ffi::{c_char, c_void};

/// whisper.cpp's context, opaque to Rust.
#[repr(C)]
pub struct WhisperContext {
    _private: [u8; 0],
}

#[repr(C)]
pub struct BrReader {
    pub ctx: *mut c_void,
    pub read: unsafe extern "C" fn(ctx: *mut c_void, out: *mut c_void, n: usize) -> usize,
    pub eof: unsafe extern "C" fn(ctx: *mut c_void) -> bool,
    pub close: unsafe extern "C" fn(ctx: *mut c_void),
}

#[repr(C)]
pub struct BrDecode {
    pub language: *const c_char,
    pub n_threads: i32,
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

#[repr(C)]
pub struct BrVad {
    pub enabled: bool,
    pub model_file: *const c_char,
    pub threshold: f32,
    pub min_speech_ms: i32,
    pub min_silence_ms: i32,
    pub max_speech_s: f32,
    pub speech_pad_ms: i32,
    pub samples_overlap_s: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BrTimings {
    pub vad_ms: f32,
    pub encode_ms: f32,
    pub decode_ms: f32,
}

pub type BrLogitsCb = Option<
    unsafe extern "C" fn(
        token_ids: *const i32,
        n_tokens: i32,
        logits: *mut f32,
        n_vocab: i32,
        user: *mut c_void,
    ),
>;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BrParamsView {
    pub strategy: i32,
    pub best_of: i32,
    pub no_context: bool,
    pub no_timestamps: bool,
    pub single_segment: bool,
    pub print_special: bool,
    pub print_progress: bool,
    pub print_realtime: bool,
    pub print_timestamps: bool,
    pub vad: bool,
    pub temperature_inc: f32,
    pub suppress_nst: bool,
    pub n_threads: i32,
}

pub const BR_OK: i32 = 0;
pub const BR_ABORTED: i32 = -1000;
pub const BR_EXCEPTION: i32 = -1001;
pub const BR_TOO_LONG: i32 = -1002;
pub const BR_INVALID: i32 = -1003;
pub const BR_DETECTOR_FAILED: i32 = -1004;

/// ggml's log levels, as `br_log_should_forward` receives them.
pub const GGML_LOG_LEVEL_DEBUG: i32 = 1;
pub const GGML_LOG_LEVEL_INFO: i32 = 2;
pub const GGML_LOG_LEVEL_WARN: i32 = 3;
pub const GGML_LOG_LEVEL_ERROR: i32 = 4;
pub const GGML_LOG_LEVEL_CONT: i32 = 5;

extern "C" {
    pub fn br_init(model: *const BrReader, use_gpu: bool, flash_attn: bool) -> *mut WhisperContext;
    pub fn br_free(ctx: *mut WhisperContext);
    pub fn br_gpu_device(buf: *mut c_char, cap: i32) -> i32;
    pub fn br_n_vocab(ctx: *mut WhisperContext) -> i32;
    pub fn br_is_multilingual(ctx: *mut WhisperContext) -> bool;
    pub fn br_token_eot(ctx: *mut WhisperContext) -> i32;
    pub fn br_token_bytes(ctx: *mut WhisperContext, id: i32, buf: *mut c_char, cap: i32) -> i32;
    pub fn br_tokenize(
        ctx: *mut WhisperContext,
        text: *const c_char,
        tokens: *mut i32,
        cap: i32,
    ) -> i32;

    #[allow(clippy::too_many_arguments)]
    pub fn br_full(
        ctx: *mut WhisperContext,
        d: *const BrDecode,
        v: *const BrVad,
        pcm: *const f32,
        n_samples: i32,
        prompt: *const i32,
        n_prompt: i32,
        cb: BrLogitsCb,
        cb_user: *mut c_void,
        abort_flag: *const i32,
        out: *mut BrTimings,
    ) -> i32;
    pub fn br_n_segments(ctx: *mut WhisperContext) -> i32;
    pub fn br_segment_text(ctx: *mut WhisperContext, i: i32) -> *const c_char;
    pub fn br_segment_span(ctx: *mut WhisperContext, i: i32, t0_ms: *mut i64, t1_ms: *mut i64);
    pub fn br_warmup(
        ctx: *mut WhisperContext,
        v: *const BrVad,
        n_threads: i32,
        encoder: bool,
    ) -> i32;
    pub fn br_params_snapshot(d: *const BrDecode, v: *const BrVad, out: *mut BrParamsView);

    pub fn br_vad_init(model: *const BrReader, n_threads: i32) -> *mut c_void;
    pub fn br_vad_spans(
        vad: *mut c_void,
        pcm: *const f32,
        n_samples: i32,
        params: *const BrVad,
        spans_ms: *mut i64,
        cap_pairs: i32,
    ) -> i32;
    pub fn br_vad_free(vad: *mut c_void);

    pub fn br_log_install();
    pub fn br_log_should_forward(level: i32, text: *const c_char) -> bool;
    pub fn br_system_info() -> *const c_char;
    pub fn br_whisper_version() -> *const c_char;
    pub fn br_sizeof(which: i32) -> usize;
}
