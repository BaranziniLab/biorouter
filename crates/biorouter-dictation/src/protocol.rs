//! The worker protocol (`docs/dictation/engine-and-models.md`, "Frame protocol" and
//! "Messages").
//!
//! Both directions carry frames of `[u32 LE json_len][json][u32 LE payload_len][payload]`.
//! The JSON is one message with a `type`; the payload is PCM s16le, mono, 16 kHz, or
//! empty. The worker's stdout carries frames only. The codec is written once: the
//! length checks are shared, with a blocking reader for the worker's threads and an
//! async reader for the supervisor. Writers take the payload as a separate slice, so a
//! segment goes from the caller's shared buffer to the pipe without a copy.
//!
//! Unknown fields are ignored and an unknown `type` reads as `Unknown`, so a newer
//! parent and an older worker of the same protocol version still talk; the version gate
//! is `hello.protocol`.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::audio::Span;
use crate::cpu::CpuFeatures;

/// The one version this build speaks.
pub const PROTOCOL_VERSION: u32 = 1;
/// The largest JSON part of a frame. A `set_bias` of 5,000 entries (DD-41) is under 1 MiB.
pub const MAX_JSON_BYTES: usize = 4 << 20;
/// The largest frame, JSON and payload together: 600 s of 16 kHz s16le is 19,200,000 B.
pub const MAX_FRAME_BYTES: usize = 32 << 20;
/// At most 25 s of audio in one `transcribe` (DD-47).
pub const MAX_TRANSCRIBE_SAMPLES: usize = crate::audio::MAX_SEGMENT_SAMPLES;
/// At most 600 s of audio in one `vad_segments` (DD-28).
pub const MAX_VAD_SAMPLES: usize = crate::audio::MAX_STREAM_SAMPLES;

/// One frame as read: the message's JSON and the payload's bytes.
#[derive(Debug, Default)]
pub struct Frame {
    pub json: Vec<u8>,
    pub payload: Vec<u8>,
}

/// Why a frame could not be read. `Display` is a fixed phrase.
#[derive(Debug)]
pub enum FrameError {
    /// The stream ended cleanly, between two frames.
    Closed,
    /// The stream ended inside a frame.
    Truncated,
    /// The JSON part is longer than [`MAX_JSON_BYTES`].
    JsonTooLarge(u64),
    /// JSON and payload together are longer than [`MAX_FRAME_BYTES`].
    FrameTooLarge(u64),
    Io(io::Error),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Closed => f.write_str("the stream ended"),
            FrameError::Truncated => f.write_str("the stream ended inside a frame"),
            FrameError::JsonTooLarge(n) => {
                write!(f, "a frame's message is {n} bytes, over the limit")
            }
            FrameError::FrameTooLarge(n) => write!(f, "a frame is {n} bytes, over the limit"),
            FrameError::Io(e) => write!(f, "the stream could not be read: {}", e.kind()),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<io::Error> for FrameError {
    fn from(e: io::Error) -> FrameError {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            FrameError::Truncated
        } else {
            FrameError::Io(e)
        }
    }
}

fn check_json_len(len: u32) -> Result<usize, FrameError> {
    let len = len as usize;
    if len > MAX_JSON_BYTES {
        return Err(FrameError::JsonTooLarge(len as u64));
    }
    Ok(len)
}

fn check_frame_len(json_len: usize, payload_len: u32) -> Result<usize, FrameError> {
    let payload_len = payload_len as usize;
    let total = json_len as u64 + payload_len as u64;
    if total > MAX_FRAME_BYTES as u64 {
        return Err(FrameError::FrameTooLarge(total));
    }
    Ok(payload_len)
}

/// Reads one frame from a blocking reader: the worker's stdin.
pub fn read_frame_blocking<R: Read>(r: &mut R) -> Result<Frame, FrameError> {
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < len.len() {
        match r.read(len.get_mut(got..).unwrap_or_default()) {
            Ok(0) if got == 0 => return Err(FrameError::Closed),
            Ok(0) => return Err(FrameError::Truncated),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    let json_len = check_json_len(u32::from_le_bytes(len))?;
    let json = read_exactly_blocking(r, json_len)?;
    r.read_exact(&mut len)?;
    let payload_len = check_frame_len(json_len, u32::from_le_bytes(len))?;
    let payload = read_exactly_blocking(r, payload_len)?;
    Ok(Frame { json, payload })
}

/// `n` bytes into a buffer allocated once, without zeroing it first.
fn read_exactly_blocking<R: Read>(r: &mut R, n: usize) -> Result<Vec<u8>, FrameError> {
    let mut buf = Vec::with_capacity(n);
    r.take(n as u64).read_to_end(&mut buf)?;
    if buf.len() != n {
        return Err(FrameError::Truncated);
    }
    Ok(buf)
}

/// Reads one frame from an async reader: the supervisor's end of the worker's stdout.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Frame, FrameError> {
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < len.len() {
        match r.read(len.get_mut(got..).unwrap_or_default()).await {
            Ok(0) if got == 0 => return Err(FrameError::Closed),
            Ok(0) => return Err(FrameError::Truncated),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    let json_len = check_json_len(u32::from_le_bytes(len))?;
    let json = read_exactly(r, json_len).await?;
    r.read_exact(&mut len).await?;
    let payload_len = check_frame_len(json_len, u32::from_le_bytes(len))?;
    let payload = read_exactly(r, payload_len).await?;
    Ok(Frame { json, payload })
}

async fn read_exactly<R: AsyncRead + Unpin>(r: &mut R, n: usize) -> Result<Vec<u8>, FrameError> {
    let mut buf = Vec::with_capacity(n);
    (&mut *r).take(n as u64).read_to_end(&mut buf).await?;
    if buf.len() != n {
        return Err(FrameError::Truncated);
    }
    Ok(buf)
}

/// Everything of a frame before its payload: `[json_len][json][payload_len]`. Refuses a
/// frame the reader on the other side would refuse.
pub fn frame_head(json: &[u8], payload_len: usize) -> Result<Vec<u8>, FrameError> {
    let json_len = u32::try_from(json.len())
        .ok()
        .filter(|n| *n as usize <= MAX_JSON_BYTES)
        .ok_or(FrameError::JsonTooLarge(json.len() as u64))?;
    let payload = u32::try_from(payload_len)
        .ok()
        .filter(|_| json.len() as u64 + payload_len as u64 <= MAX_FRAME_BYTES as u64)
        .ok_or(FrameError::FrameTooLarge(
            json.len() as u64 + payload_len as u64,
        ))?;
    let mut head = Vec::with_capacity(8 + json.len());
    head.extend_from_slice(&json_len.to_le_bytes());
    head.extend_from_slice(json);
    head.extend_from_slice(&payload.to_le_bytes());
    Ok(head)
}

fn invalid(e: FrameError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, e.to_string())
}

/// Writes one frame and flushes it.
pub fn write_frame_blocking<W: Write>(w: &mut W, json: &[u8], payload: &[u8]) -> io::Result<()> {
    let head = frame_head(json, payload.len()).map_err(invalid)?;
    w.write_all(&head)?;
    w.write_all(payload)?;
    w.flush()
}

/// Writes one frame and flushes it.
pub async fn write_frame<W: AsyncWrite + Unpin>(
    w: &mut W,
    json: &[u8],
    payload: &[u8],
) -> io::Result<()> {
    let head = frame_head(json, payload.len()).map_err(invalid)?;
    w.write_all(&head).await?;
    w.write_all(payload).await?;
    w.flush().await
}

/// A message's JSON. Serializing these types cannot fail: every map key is a string.
pub fn to_json<T: Serialize>(message: &T) -> Vec<u8> {
    serde_json::to_vec(message).unwrap_or_default()
}

/// Messages from the parent to the worker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToWorker {
    Load(Load),
    SetBias(SetBias),
    /// Payload: PCM, at most 25 s.
    Transcribe(Transcribe),
    /// Payload: PCM, at most 600 s.
    VadSegments(VadSegments),
    Cancel {
        id: u64,
    },
    Shutdown,
    #[serde(other)]
    Unknown,
}

/// Messages from the worker to the parent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FromWorker {
    Hello(Hello),
    Loaded(Loaded),
    BiasReady(BiasReady),
    Result(TranscribeResult),
    Segments(Segments),
    Error(ErrorMsg),
    #[serde(other)]
    Unknown,
}

/// The worker's first frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    /// The vendored whisper.cpp commit, eight hex digits.
    pub whisper: String,
    pub ggml: String,
    /// `["metal", "cpu"]` on Apple Silicon, else `["cpu"]`.
    pub backends: Vec<String>,
    /// The features the worker's own CPUID view reports.
    pub cpu: CpuFlags,
    pub isa: String,
    /// `whisper_print_system_info()`; empty when the CPU gate refused, because calling
    /// into ggml on such a processor could itself fault.
    pub system_info: String,
}

/// [`CpuFeatures`] on the wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuFlags {
    pub avx2: bool,
    pub fma: bool,
    pub f16c: bool,
    pub bmi2: bool,
    pub dotprod: bool,
}

impl From<CpuFeatures> for CpuFlags {
    fn from(f: CpuFeatures) -> CpuFlags {
        CpuFlags {
            avx2: f.avx2,
            fma: f.fma,
            f16c: f.f16c,
            bmi2: f.bmi2,
            dotprod: f.dotprod,
        }
    }
}

impl From<CpuFlags> for CpuFeatures {
    fn from(f: CpuFlags) -> CpuFeatures {
        CpuFeatures {
            avx2: f.avx2,
            fma: f.fma,
            f16c: f.f16c,
            bmi2: f.bmi2,
            dotprod: f.dotprod,
        }
    }
}

/// Open a model. The parent has verified its SHA-256 (DD-17); the worker checks its
/// length and its header, never its hash, which would add about 0.3 s per load.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Load {
    pub model_path: String,
    /// The key of the loaded model: a second `load` with the same hash and backend is
    /// answered from memory.
    pub model_sha256: String,
    pub model_size: u64,
    /// The detector's folder, which becomes the worker's current directory.
    pub vad_dir: String,
    /// The detector's bare ASCII file name inside `vad_dir`.
    pub vad_file: String,
    pub use_gpu: bool,
    pub n_threads: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loaded {
    /// The model and the detectors, from the `load` frame to ready.
    pub load_ms: u64,
    /// The warm up that followed: the integrated detector and, on Metal, one encoder
    /// pass. Reported apart from `load_ms`; a worker that predates it sends none.
    #[serde(default)]
    pub warmup_ms: u64,
    /// `"metal"` or `"cpu"`.
    pub backend: String,
    pub device: Option<String>,
    /// Peak resident size after the load.
    pub rss_bytes: Option<u64>,
    pub n_vocab: i32,
    pub multilingual: bool,
}

/// PR-14 builds the decoder bonus from this; until then the worker refuses it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SetBias {
    pub revision: u64,
    pub scope: String,
    pub entries: Vec<BiasEntry>,
    pub params: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BiasEntry {
    pub written: String,
    /// `"auto"` or `"always"`.
    pub bias: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BiasReady {
    pub revision: u64,
    pub scope: String,
    pub nodes: u64,
    pub build_ms: u64,
}

/// Which installed bias table a request uses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BiasRef {
    pub revision: u64,
    pub scope: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transcribe {
    pub id: u64,
    pub bias: Option<BiasRef>,
    /// Ordered, best last.
    pub prompt_terms: Vec<String>,
    pub prompt_budget_tokens: u32,
    pub language: String,
    pub decode: DecodeOptions,
    pub vad: VadOptions,
}

/// The token budget of a prompt (DD-47).
pub const PROMPT_BUDGET_TOKENS: u32 = 150;

/// The DD-47 decoding parameters on the wire.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecodeOptions {
    /// `"greedy"`, the only strategy the shim runs.
    pub strategy: String,
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
    pub n_threads: u32,
    /// 0 is the full 30 s window (DD-47; lever L-8 of the M1 design measures others).
    pub audio_ctx: i32,
}

impl DecodeOptions {
    /// The DD-47 values. The worker's own `engine::decode_params` holds the same values,
    /// and a test holds the two together.
    pub fn dd47(n_threads: u32) -> DecodeOptions {
        DecodeOptions {
            strategy: "greedy".to_string(),
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
            n_threads,
            audio_ctx: 0,
        }
    }
}

/// The DD-48 voice detection parameters on the wire.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VadOptions {
    /// The integrated detector inside a decode; `vad_segments` always runs its own.
    pub enabled: bool,
    pub threshold: f32,
    pub min_speech_ms: i32,
    pub min_silence_ms: i32,
    pub speech_pad_ms: i32,
    /// `null` is upstream's unlimited default; the parent splits at 25 s.
    pub max_speech_s: Option<f32>,
    pub samples_overlap_s: f32,
}

impl VadOptions {
    /// The DD-48 values.
    pub fn dd48() -> VadOptions {
        VadOptions {
            enabled: true,
            threshold: 0.5,
            min_speech_ms: 250,
            min_silence_ms: 100,
            speech_pad_ms: 30,
            max_speech_s: None,
            samples_overlap_s: 0.1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranscribeResult {
    pub id: u64,
    /// Every segment's text, joined and trimmed; timestamps never appear in it.
    pub text: String,
    pub segments: Vec<SegmentText>,
    /// How many terms from the tail of `prompt_terms` made the token budget.
    pub prompt_terms_used: u32,
    pub timings: Timings,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentText {
    pub t0_ms: u64,
    pub t1_ms: u64,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Timings {
    pub vad_ms: f32,
    pub encode_ms: f32,
    pub decode_ms: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VadSegments {
    pub id: u64,
    pub vad: VadOptions,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segments {
    pub id: u64,
    pub spans: Vec<Span>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    ModelLoadFailed,
    BadAudio,
    Cancelled,
    Internal,
    #[serde(other)]
    Unknown,
}

/// An error from the worker. `message` is a fixed English phrase: never a transcript, a
/// prompt, a path or audio (P-12).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorMsg {
    pub id: Option<u64>,
    pub code: ErrorCode,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip_blocking(json: &[u8], payload: &[u8]) -> Frame {
        let mut wire = Vec::new();
        write_frame_blocking(&mut wire, json, payload).expect("write");
        assert_eq!(wire.len(), 8 + json.len() + payload.len());
        let mut reader = wire.as_slice();
        let frame = read_frame_blocking(&mut reader).expect("read");
        assert!(matches!(
            read_frame_blocking(&mut reader),
            Err(FrameError::Closed)
        ));
        frame
    }

    #[test]
    fn frames_round_trip_on_blocking_streams() {
        let frame = round_trip_blocking(br#"{"type":"shutdown"}"#, &[]);
        assert_eq!(frame.json, br#"{"type":"shutdown"}"#);
        assert!(frame.payload.is_empty());
        let pcm: Vec<u8> = (0..=255u8).cycle().take(800_000).collect();
        let frame = round_trip_blocking(b"{}", &pcm);
        assert_eq!(frame.payload, pcm);
    }

    #[tokio::test]
    async fn frames_round_trip_on_async_streams() {
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        let pcm: Vec<u8> = (0..=255u8).cycle().take(800_000).collect();
        let sent = pcm.clone();
        let writer = tokio::spawn(async move {
            write_frame(&mut a, br#"{"type":"load"}"#, &[])
                .await
                .expect("write");
            write_frame(&mut a, b"{}", &sent).await.expect("write");
        });
        let first = read_frame(&mut b).await.expect("first");
        assert_eq!(first.json, br#"{"type":"load"}"#);
        let second = read_frame(&mut b).await.expect("second");
        assert_eq!(second.payload, pcm);
        writer.await.expect("writer");
        assert!(matches!(read_frame(&mut b).await, Err(FrameError::Closed)));
    }

    #[test]
    fn the_size_caps_hold_in_both_directions() {
        // A JSON part of 4 MiB is accepted, one byte more is refused before it is read.
        let at_cap = vec![b' '; MAX_JSON_BYTES];
        assert!(frame_head(&at_cap, 0).is_ok());
        let over = vec![b' '; MAX_JSON_BYTES + 1];
        assert!(matches!(
            frame_head(&over, 0),
            Err(FrameError::JsonTooLarge(_))
        ));
        assert!(write_frame_blocking(&mut Vec::new(), &over, &[]).is_err());

        let mut wire = Vec::new();
        wire.extend_from_slice(&((MAX_JSON_BYTES as u32) + 1).to_le_bytes());
        assert!(matches!(
            read_frame_blocking(&mut wire.as_slice()),
            Err(FrameError::JsonTooLarge(_))
        ));

        // 32 MiB for JSON and payload together: the header alone decides, so a frame
        // that claims more is refused without allocating it.
        let mut wire = Vec::new();
        wire.extend_from_slice(&2u32.to_le_bytes());
        wire.extend_from_slice(b"{}");
        wire.extend_from_slice(&((MAX_FRAME_BYTES as u32) - 1).to_le_bytes());
        assert!(matches!(
            read_frame_blocking(&mut wire.as_slice()),
            Err(FrameError::FrameTooLarge(_))
        ));
        assert!(matches!(
            frame_head(b"{}", MAX_FRAME_BYTES - 1),
            Err(FrameError::FrameTooLarge(_))
        ));
        assert!(frame_head(b"{}", MAX_FRAME_BYTES - 2).is_ok());
        let mut wire = Vec::new();
        wire.extend_from_slice(&2u32.to_le_bytes());
        wire.extend_from_slice(b"{}");
        wire.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            read_frame_blocking(&mut wire.as_slice()),
            Err(FrameError::FrameTooLarge(_))
        ));
    }

    #[tokio::test]
    async fn the_async_reader_applies_the_same_caps() {
        let mut wire = Vec::new();
        wire.extend_from_slice(&((MAX_JSON_BYTES as u32) + 1).to_le_bytes());
        assert!(matches!(
            read_frame(&mut wire.as_slice()).await,
            Err(FrameError::JsonTooLarge(_))
        ));
        let mut wire = Vec::new();
        wire.extend_from_slice(&2u32.to_le_bytes());
        wire.extend_from_slice(b"{}");
        wire.extend_from_slice(&(MAX_FRAME_BYTES as u32).to_le_bytes());
        assert!(matches!(
            read_frame(&mut wire.as_slice()).await,
            Err(FrameError::FrameTooLarge(_))
        ));
    }

    #[test]
    fn a_stream_cut_inside_a_frame_is_truncated() {
        let mut wire = Vec::new();
        write_frame_blocking(&mut wire, b"{\"type\":\"shutdown\"}", &[1, 2, 3, 4]).expect("write");
        for cut in [1, 3, 5, 25, wire.len() - 1] {
            let partial = wire.get(..cut).expect("cut");
            assert!(
                matches!(
                    read_frame_blocking(&mut &partial[..]),
                    Err(FrameError::Truncated)
                ),
                "cut at {cut}"
            );
        }
        assert!(matches!(
            read_frame_blocking(&mut &[][..]),
            Err(FrameError::Closed)
        ));
    }

    fn golden<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(
        message: T,
        expected: serde_json::Value,
    ) {
        let written: serde_json::Value =
            serde_json::from_slice(&to_json(&message)).expect("valid JSON");
        assert_eq!(written, expected);
        let read: T = serde_json::from_value(expected).expect("reads back");
        assert_eq!(read, message);
    }

    /// Every message type, with the field names of engine-and-models.md "Messages".
    #[test]
    fn every_message_has_the_documented_fields() {
        golden(
            FromWorker::Hello(Hello {
                protocol: 1,
                whisper: "4da27c41".into(),
                ggml: "0.28.0".into(),
                backends: vec!["metal".into(), "cpu".into()],
                cpu: CpuFlags {
                    dotprod: true,
                    ..CpuFlags::default()
                },
                isa: "armv8.2-a+dotprod+fp16".into(),
                system_info: "WHISPER : COREML = 0 | ".into(),
            }),
            json!({"type":"hello","protocol":1,"whisper":"4da27c41","ggml":"0.28.0",
                   "backends":["metal","cpu"],
                   "cpu":{"avx2":false,"fma":false,"f16c":false,"bmi2":false,"dotprod":true},
                   "isa":"armv8.2-a+dotprod+fp16","system_info":"WHISPER : COREML = 0 | "}),
        );
        golden(
            ToWorker::Load(Load {
                model_path: "/m/ggml-large-v3-turbo-q5_0.bin".into(),
                model_sha256: "3942".into(),
                model_size: 574_041_195,
                vad_dir: "/m/silero-vad-v6.2.0".into(),
                vad_file: "ggml-silero-v6.2.0.bin".into(),
                use_gpu: true,
                n_threads: 8,
            }),
            json!({"type":"load","model_path":"/m/ggml-large-v3-turbo-q5_0.bin",
                   "model_sha256":"3942","model_size":574_041_195u64,
                   "vad_dir":"/m/silero-vad-v6.2.0","vad_file":"ggml-silero-v6.2.0.bin",
                   "use_gpu":true,"n_threads":8}),
        );
        golden(
            FromWorker::Loaded(Loaded {
                load_ms: 420,
                warmup_ms: 260,
                backend: "metal".into(),
                device: Some("Apple M4 Max".into()),
                rss_bytes: Some(1_010_000_000),
                n_vocab: 51_866,
                multilingual: true,
            }),
            json!({"type":"loaded","load_ms":420,"warmup_ms":260,"backend":"metal",
                   "device":"Apple M4 Max","rss_bytes":1_010_000_000u64,"n_vocab":51_866,
                   "multilingual":true}),
        );
        golden(
            ToWorker::SetBias(SetBias {
                revision: 4,
                scope: "public".into(),
                entries: vec![BiasEntry {
                    written: "EPAS1".into(),
                    bias: "auto".into(),
                }],
                params: json!({"bonus": 2.0}),
            }),
            json!({"type":"set_bias","revision":4,"scope":"public",
                   "entries":[{"written":"EPAS1","bias":"auto"}],"params":{"bonus":2.0}}),
        );
        golden(
            FromWorker::BiasReady(BiasReady {
                revision: 4,
                scope: "public".into(),
                nodes: 120,
                build_ms: 3,
            }),
            json!({"type":"bias_ready","revision":4,"scope":"public","nodes":120,"build_ms":3}),
        );
        golden(
            ToWorker::Transcribe(Transcribe {
                id: 7,
                bias: None,
                prompt_terms: vec![],
                prompt_budget_tokens: PROMPT_BUDGET_TOKENS,
                language: "en".into(),
                decode: DecodeOptions::dd47(8),
                vad: VadOptions::dd48(),
            }),
            json!({"type":"transcribe","id":7,"bias":null,"prompt_terms":[],
                   "prompt_budget_tokens":150,"language":"en",
                   "decode":{"strategy":"greedy","best_of":5,"temperature":0.0,
                             "temperature_inc":0.2,"entropy_thold":2.4,"logprob_thold":-1.0,
                             "no_speech_thold":0.6,"translate":false,"no_context":true,
                             "no_timestamps":false,"single_segment":false,
                             "suppress_blank":true,"suppress_nst":false,"n_threads":8,
                             "audio_ctx":0},
                   "vad":{"enabled":true,"threshold":0.5,"min_speech_ms":250,
                          "min_silence_ms":100,"speech_pad_ms":30,"max_speech_s":null,
                          "samples_overlap_s":0.1}}),
        );
        golden(
            FromWorker::Result(TranscribeResult {
                id: 7,
                text: "Hello there.".into(),
                segments: vec![SegmentText {
                    t0_ms: 0,
                    t1_ms: 4_800,
                    text: "Hello there.".into(),
                }],
                prompt_terms_used: 0,
                timings: Timings {
                    vad_ms: 14.0,
                    encode_ms: 120.0,
                    decode_ms: 140.0,
                },
            }),
            json!({"type":"result","id":7,"text":"Hello there.",
                   "segments":[{"t0_ms":0,"t1_ms":4800,"text":"Hello there."}],
                   "prompt_terms_used":0,
                   "timings":{"vad_ms":14.0,"encode_ms":120.0,"decode_ms":140.0}}),
        );
        golden(
            ToWorker::VadSegments(VadSegments {
                id: 8,
                vad: VadOptions::dd48(),
            }),
            json!({"type":"vad_segments","id":8,
                   "vad":{"enabled":true,"threshold":0.5,"min_speech_ms":250,
                          "min_silence_ms":100,"speech_pad_ms":30,"max_speech_s":null,
                          "samples_overlap_s":0.1}}),
        );
        golden(
            FromWorker::Segments(Segments {
                id: 8,
                spans: vec![Span {
                    t0_ms: 120,
                    t1_ms: 2_900,
                }],
            }),
            json!({"type":"segments","id":8,"spans":[{"t0_ms":120,"t1_ms":2900}]}),
        );
        golden(ToWorker::Cancel { id: 7 }, json!({"type":"cancel","id":7}));
        golden(ToWorker::Shutdown, json!({"type":"shutdown"}));
        golden(
            FromWorker::Error(ErrorMsg {
                id: None,
                code: ErrorCode::ModelLoadFailed,
                message: "model file size does not match".into(),
            }),
            json!({"type":"error","id":null,"code":"model_load_failed",
                   "message":"model file size does not match"}),
        );
        for (code, name) in [
            (ErrorCode::BadAudio, "bad_audio"),
            (ErrorCode::Cancelled, "cancelled"),
            (ErrorCode::Internal, "internal"),
        ] {
            assert_eq!(serde_json::to_value(code).expect("code"), json!(name));
        }
    }

    #[test]
    fn unknown_types_and_fields_are_tolerated() {
        let read: ToWorker =
            serde_json::from_str(r#"{"type":"warm_up","id":3}"#).expect("an unknown type");
        assert_eq!(read, ToWorker::Unknown);
        let read: FromWorker =
            serde_json::from_str(r#"{"type":"progress","percent":40}"#).expect("unknown");
        assert_eq!(read, FromWorker::Unknown);
        let read: ToWorker = serde_json::from_str(r#"{"type":"cancel","id":3,"reason":"x"}"#)
            .expect("an unknown field");
        assert_eq!(read, ToWorker::Cancel { id: 3 });
        let read: ErrorMsg =
            serde_json::from_str(r#"{"id":1,"code":"out_of_memory","message":"m"}"#)
                .expect("an unknown code");
        assert_eq!(read.code, ErrorCode::Unknown);
        // A worker from before `warmup_ms` still reads.
        let read: Loaded = serde_json::from_str(
            r#"{"load_ms":1,"backend":"cpu","device":null,"rss_bytes":null,"n_vocab":51864,"multilingual":false}"#,
        )
        .expect("no warmup_ms");
        assert_eq!(read.warmup_ms, 0);
    }
}
