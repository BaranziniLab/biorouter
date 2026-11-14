//! A recording from the capture child to text (DD-50, DD-73; `docs/dictation/surfaces.md`,
//! "Limits").
//!
//! [`Recorder`] holds the samples and watches them: exact zeros for the first 1.5 s are the
//! permission path of step 10 (macOS and Windows deliver zeros when the microphone is
//! denied), 3 s below the noise floor before anything was heard is MSG-NO-AUDIO-YET while
//! the recording goes on, 570 s warns and 600 s stops, `--seconds` stops sooner, and a
//! recording under 0.5 s is too short. [`Failures`] pauses dictation after three failures
//! within 10 s, and [`RetryHold`] keeps a failed recording for `/dictate retry` for at most
//! 5 minutes, in memory only, for the terminal front ends (PR-26).
//!
//! [`LiveDecode`] decodes while the person is still speaking, as the daemon's streams do:
//! once 8 s lie past the decoded mark, the voice detector reads the undecoded audio and
//! every closed stretch of speech goes to the warm worker, so at stop only the open tail is
//! left to decode, and that tail goes straight to `transcribe` when it is under 25 s. It
//! talks to the engine through [`Engine`], so its planning is tested without one.
//!
//! P-1: the recording ([`Recorder`], then [`Pcm`]) and every copy of part of it made for a
//! detector pass or a decode are zeroed when dropped, on every way out: a stop, a cancel, a
//! failure. The recorder reserves its whole limit up front, so it never reallocates and
//! leaves no unzeroed old buffer behind.

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use biorouter::dictation::audio::{
    plan_closed_segments, plan_final_segments, MAX_SEGMENT_MS, SAMPLES_PER_MS,
    STREAM_VAD_TRIGGER_MS,
};
use biorouter::dictation::recognize::RetryReason;
use biorouter::dictation::service::{SegmentDecode, TranscribeRequest};
use biorouter::dictation::{DictationError, LoadSpec, PcmSlice, Span};
pub use biorouter_dictation_capture::wipe;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// The longest recording: 600 s (DD-28).
pub const MAX_RECORDING_MS: u64 = 600_000;
/// MSG-CAP-WARN, 30 s before the cap.
pub const CAP_WARN_MS: u64 = 570_000;
/// Exact zeros for this long are the permission path.
pub const ZERO_WINDOW_MS: u64 = 1_500;
/// Below the noise floor for this long, before anything was heard, is MSG-NO-AUDIO-YET.
pub const QUIET_MS: u64 = 3_000;
/// About -40 dBFS.
pub const NOISE_FLOOR: f32 = 0.01;
/// Under this is MSG-TOO-SHORT.
pub const MIN_RECORDING_MS: u64 = 500;
/// Three failures within [`PAUSE_WINDOW`] pause dictation for the session (MSG-PAUSED).
pub const PAUSE_AFTER: usize = 3;
pub const PAUSE_WINDOW: Duration = Duration::from_secs(10);
/// How long a failed recording is kept for `/dictate retry`.
pub const RETRY_KEEP: Duration = Duration::from_secs(300);
/// The longest detector pass while the person speaks, as the daemon's streams cut it.
pub const MAX_DETECTOR_PASS_MS: u64 = 60_000;
/// How much audio before a window's end a pass that found no speech keeps unread.
const SILENCE_KEPT_MS: u64 = 1_000;

/// What a push of samples brought up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    /// MSG-NO-AUDIO-YET, once; the recording goes on.
    NoAudioYet,
    /// MSG-CAP-WARN, once, at 570 s.
    CapWarn,
    /// Stop now: the 600 s cap (MSG-CAP-STOPPED) or `--seconds`.
    Stop(StopReason),
    /// Exact zeros for 1.5 s: step 10's permission sentence, and the recording ends.
    Denied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    Cap,
    Seconds,
}

/// How a recording ended without audio to transcribe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// Under 0.5 s: MSG-TOO-SHORT, exit 3.
    TooShort,
    /// Every sample exactly zero: nothing to decode, exit 3.
    Silent,
}

/// A recording's samples, 16 kHz mono s16, and what they show.
#[derive(Debug)]
pub struct Recorder {
    pcm: Vec<i16>,
    limit: usize,
    limit_reason: StopReason,
    only_zeros: bool,
    zero_decided: bool,
    heard: bool,
    above_floor: bool,
    told_quiet: bool,
    warned: bool,
    stopped: bool,
}

impl Recorder {
    /// `seconds` is `--seconds`, at most 600.
    pub fn new(seconds: Option<u32>) -> Recorder {
        let cap = (MAX_RECORDING_MS * SAMPLES_PER_MS) as usize;
        let (limit, limit_reason) = match seconds {
            Some(s) if (u64::from(s) * 1_000) < MAX_RECORDING_MS => (
                (u64::from(s) * 1_000 * SAMPLES_PER_MS) as usize,
                StopReason::Seconds,
            ),
            _ => (cap, StopReason::Cap),
        };
        Recorder {
            // The whole limit at once (at most 19.2 MB, mapped lazily): a buffer that grew
            // would free its old copy of the audio without zeroing it.
            pcm: Vec::with_capacity(limit),
            limit,
            limit_reason,
            only_zeros: true,
            zero_decided: false,
            heard: false,
            above_floor: false,
            told_quiet: false,
            warned: false,
            stopped: false,
        }
    }

    /// Adds `samples`, f32 in [-1, 1]. Samples past the limit are dropped.
    pub fn push(&mut self, samples: &[f32]) -> Vec<Notice> {
        let mut notices = Vec::new();
        if self.stopped {
            return notices;
        }
        let room = self.limit.saturating_sub(self.pcm.len());
        let take = samples.len().min(room);
        let zero_window = (ZERO_WINDOW_MS * SAMPLES_PER_MS) as usize;
        let quiet_window = (QUIET_MS * SAMPLES_PER_MS) as usize;
        for &s in &samples[..take] {
            if s != 0.0 {
                self.heard = true;
                if self.pcm.len() < zero_window {
                    self.only_zeros = false;
                }
            }
            if s.abs() >= NOISE_FLOOR {
                self.above_floor = true;
            }
            self.pcm
                .push((s.clamp(-1.0, 1.0) * 32_767.0).round() as i16);
        }
        if !self.zero_decided && self.pcm.len() >= zero_window {
            self.zero_decided = true;
            if self.only_zeros {
                self.stopped = true;
                notices.push(Notice::Denied);
                return notices;
            }
        }
        if !self.told_quiet && !self.above_floor && self.pcm.len() >= quiet_window {
            self.told_quiet = true;
            notices.push(Notice::NoAudioYet);
        }
        let warn_at = (CAP_WARN_MS * SAMPLES_PER_MS) as usize;
        if !self.warned && self.limit_reason == StopReason::Cap && self.pcm.len() >= warn_at {
            self.warned = true;
            notices.push(Notice::CapWarn);
        }
        if self.pcm.len() >= self.limit {
            self.stopped = true;
            notices.push(Notice::Stop(self.limit_reason));
        }
        notices
    }

    pub fn duration_ms(&self) -> u64 {
        self.pcm.len() as u64 / SAMPLES_PER_MS
    }

    pub fn pcm(&self) -> &[i16] {
        &self.pcm
    }

    /// Whether any sample was above exact zero: the line says "Starting microphone…" until
    /// then.
    pub fn heard_anything(&self) -> bool {
        self.heard
    }

    /// The samples to transcribe, or why there are none.
    pub fn finish(mut self) -> Result<Pcm, Ended> {
        if self.duration_ms() < MIN_RECORDING_MS {
            return Err(Ended::TooShort);
        }
        if !self.heard {
            return Err(Ended::Silent);
        }
        Ok(Pcm(std::mem::take(&mut self.pcm)))
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // P-1: the audio does not outlive the recording in freed memory.
        wipe(&mut self.pcm);
    }
}

/// A finished recording, 16 kHz mono s16, zeroed when dropped, whether the text came out,
/// the decode was cancelled or it failed (P-1).
#[derive(Debug)]
pub struct Pcm(Vec<i16>);

impl std::ops::Deref for Pcm {
    type Target = [i16];

    fn deref(&self) -> &[i16] {
        &self.0
    }
}

impl Drop for Pcm {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

/// Failures within [`PAUSE_WINDOW`]: the third pauses dictation for the session.
#[derive(Debug, Default)]
pub struct Failures {
    at: Vec<Instant>,
    paused: bool,
}

impl Failures {
    /// Records a failure at `now`; true when dictation is now paused (MSG-PAUSED).
    pub fn record(&mut self, now: Instant) -> bool {
        self.at
            .retain(|t| now.saturating_duration_since(*t) < PAUSE_WINDOW);
        self.at.push(now);
        if self.at.len() >= PAUSE_AFTER {
            self.paused = true;
        }
        self.paused
    }

    pub fn paused(&self) -> bool {
        self.paused
    }
}

/// A failed recording, kept in memory for `/dictate retry` for at most [`RETRY_KEEP`],
/// and dropped on the next recording, on cancel or on quit. Never written to disk.
#[derive(Debug, Default)]
pub struct RetryHold {
    held: Option<(Vec<i16>, Instant)>,
}

impl RetryHold {
    pub fn keep(&mut self, pcm: Vec<i16>, now: Instant) {
        self.clear();
        self.held = Some((pcm, now));
    }

    /// The recording, when it is still within its 5 minutes.
    pub fn take(&mut self, now: Instant) -> Option<Vec<i16>> {
        let (pcm, at) = self.held.take()?;
        if now.saturating_duration_since(at) <= RETRY_KEEP {
            Some(pcm)
        } else {
            let mut pcm = pcm;
            wipe(&mut pcm);
            None
        }
    }

    pub fn clear(&mut self) {
        if let Some((mut pcm, _)) = self.held.take() {
            wipe(&mut pcm);
        }
    }
}

impl Drop for RetryHold {
    fn drop(&mut self) {
        self.clear();
    }
}

/// What [`LiveDecode`] needs of the engine.
#[async_trait::async_trait]
pub trait Engine: Send + Sync + 'static {
    async fn vad(&self, pcm: PcmSlice) -> Result<Vec<Span>, DictationError>;
    async fn decode(
        &self,
        pcm: PcmSlice,
        speech_ms: Option<u64>,
        request: &TranscribeRequest,
    ) -> Result<SegmentDecode, DictationError>;
}

/// The process's own worker, through the service (one per process).
pub struct ServiceEngine {
    pub spec: LoadSpec,
    /// The recording's key in the worker's round robin; retries take `stream + 1`.
    pub stream: u64,
}

#[async_trait::async_trait]
impl Engine for ServiceEngine {
    async fn vad(&self, pcm: PcmSlice) -> Result<Vec<Span>, DictationError> {
        biorouter::dictation::service()
            .vad_segments_with(self.spec.clone(), pcm)
            .await
    }

    async fn decode(
        &self,
        pcm: PcmSlice,
        speech_ms: Option<u64>,
        request: &TranscribeRequest,
    ) -> Result<SegmentDecode, DictationError> {
        biorouter::dictation::service()
            .decode_segment(self.spec.clone(), self.stream, pcm, speech_ms, request)
            .await
    }
}

/// Environment only, and read only by a debug build: the text every decode answers with
/// in place of the engine's. The vendored `for-tests` model hears nothing, so without it
/// `biorouter dictate`'s review, its pipe and its clipboard fallback could be tested only
/// with a real speech model. The model is still resolved and loaded as usual; only the
/// decode is answered here. A release build ignores the variable.
pub const TEST_TEXT_ENV: &str = "BIOROUTER_DICTATION_TEST_TEXT";

/// [`TEST_TEXT_ENV`]'s text, in a debug build.
pub fn test_text() -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var(TEST_TEXT_ENV)
        .ok()
        .filter(|text| !text.trim().is_empty())
}

/// The engine of [`TEST_TEXT_ENV`]: the detector hears speech throughout, and every decode
/// answers with the same text.
pub struct FixedText {
    pub text: String,
}

#[async_trait::async_trait]
impl Engine for FixedText {
    async fn vad(&self, pcm: PcmSlice) -> Result<Vec<Span>, DictationError> {
        Ok(vec![Span {
            t0_ms: 0,
            t1_ms: pcm.duration_ms(),
        }])
    }

    async fn decode(
        &self,
        pcm: PcmSlice,
        _speech_ms: Option<u64>,
        _request: &TranscribeRequest,
    ) -> Result<SegmentDecode, DictationError> {
        Ok(SegmentDecode {
            pieces: vec![biorouter_dictation::protocol::SegmentText {
                t0_ms: 0,
                t1_ms: pcm.duration_ms(),
                text: self.text.clone(),
            }],
            ..SegmentDecode::default()
        })
    }
}

/// One piece of text, its times in the recording's own milliseconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub t0_ms: u64,
    pub t1_ms: u64,
    pub text: String,
}

/// Everything decoded for one recording.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decoded {
    pub pieces: Vec<Piece>,
    pub retried: Option<RetryReason>,
    /// Segments decoded, the tail's among them.
    pub segments: u32,
    /// Segments decoded before the stop.
    pub segments_while_speaking: u32,
    pub encode_ms: u64,
    pub decode_ms: u64,
    /// From the stop to the tail's last decode returning.
    pub tail_ms: u64,
    /// Seconds of audio left for the tail.
    pub tail_audio_s: f64,
}

impl Decoded {
    /// The engine's text: the pieces joined, with whole outputs that are only tags removed.
    pub fn raw_text(&self) -> String {
        use biorouter::dictation::recognize::text;
        text::filter_whole_output(&text::join(self.pieces.iter().map(|p| p.text.as_str())))
    }
}

/// A copy of part of the recording for one detector pass or decode, zeroed when the last
/// holder drops it (P-1): after its decode, or when a pass is dropped at the stop, or when
/// a queued job is dropped with the decoder.
struct AudioCopy(Arc<[i16]>);

impl AudioCopy {
    fn slice(&self) -> PcmSlice {
        PcmSlice::whole(Arc::clone(&self.0))
    }

    fn len(&self) -> usize {
        self.0.len()
    }
}

impl Drop for AudioCopy {
    fn drop(&mut self) {
        // The engine holds no clone once its call returned; one that still did would keep
        // the samples alive, and they are zeroed when the copy here is the last.
        if let Some(samples) = Arc::get_mut(&mut self.0) {
            wipe(samples);
        }
    }
}

enum Job {
    Segment {
        pcm: AudioCopy,
        offset_ms: u64,
        speech_ms: u64,
    },
    Tail {
        pcm: AudioCopy,
        offset_ms: u64,
        open_speech_ms: u64,
        stopped_at: Instant,
    },
}

type Pass = JoinHandle<(Range<u64>, Result<Vec<Span>, DictationError>)>;

/// Decodes a recording while it is being made (DD-50).
pub struct LiveDecode {
    engine: Arc<dyn Engine>,
    jobs: Option<mpsc::UnboundedSender<Job>>,
    decoder: JoinHandle<Result<Decoded, DictationError>>,
    mark_ms: u64,
    stalled_at_ms: Option<u64>,
    open_speech_ms: u64,
    pass: Option<Pass>,
    detector_failed: bool,
}

impl LiveDecode {
    /// `request` resolves to the decode request once the dictionary is prepared; the first
    /// decode waits for it, the recording does not.
    pub fn new(
        engine: Arc<dyn Engine>,
        request: impl std::future::Future<Output = TranscribeRequest> + Send + 'static,
    ) -> LiveDecode {
        let (tx, rx) = mpsc::unbounded_channel();
        let decoder = tokio::spawn(decode_jobs(Arc::clone(&engine), request, rx));
        LiveDecode {
            engine,
            jobs: Some(tx),
            decoder,
            mark_ms: 0,
            stalled_at_ms: None,
            open_speech_ms: 0,
            pass: None,
            detector_failed: false,
        }
    }

    /// The audio up to which every closed segment is queued.
    pub fn mark_ms(&self) -> u64 {
        self.mark_ms
    }

    /// After new audio: takes a detector pass that finished and queues what it closed, then
    /// starts the next pass when one is due. Never waits.
    pub fn poll(&mut self, pcm: &[i16]) {
        use futures::FutureExt;
        if self.pass.as_ref().is_some_and(JoinHandle::is_finished) {
            if let Some(Some(Ok((window, found)))) = self.pass.take().map(|p| p.now_or_never()) {
                match found {
                    Ok(spans) => self.accept(pcm, &window, spans),
                    // A detector that failed here fails the tail too, which reports it;
                    // the audio stays for the tail meanwhile.
                    Err(_) => self.detector_failed = true,
                }
            }
        }
        if self.pass.is_some() || self.detector_failed {
            return;
        }
        let end_ms = pcm.len() as u64 / SAMPLES_PER_MS;
        let Some(window) = due_window(self.mark_ms, end_ms, self.stalled_at_ms) else {
            return;
        };
        let audio = copy_ms(pcm, &window);
        let engine = Arc::clone(&self.engine);
        self.pass = Some(tokio::spawn(async move {
            let found = engine.vad(audio.slice()).await;
            drop(audio);
            (window, found)
        }));
    }

    fn accept(&mut self, pcm: &[i16], window: &Range<u64>, spans: Vec<Span>) {
        let spans: Vec<Span> = spans
            .into_iter()
            .map(|s| Span {
                t0_ms: s.t0_ms + window.start,
                t1_ms: (s.t1_ms + window.start).min(window.end),
            })
            .filter(|s| s.t0_ms < s.t1_ms)
            .collect();
        let before = self.mark_ms;
        let (closed, mark) = plan_pass(&spans, before, window.end);
        for range in biorouter::dictation::file::pack(closed) {
            let job = Job::Segment {
                pcm: copy_ms(pcm, &range),
                offset_ms: range.start,
                speech_ms: speech_within(&spans, &range),
            };
            if let Some(jobs) = &self.jobs {
                let _ = jobs.send(job);
            }
        }
        self.mark_ms = before.max(mark);
        self.stalled_at_ms = (self.mark_ms == before).then_some(window.end);
        self.open_speech_ms = speech_within(&spans, &(self.mark_ms..window.end));
    }

    /// The stop: a detector pass still running is dropped, the open tail queued as one job,
    /// and the decoded pieces awaited.
    pub async fn finish(mut self, pcm: &[i16]) -> Result<Decoded, DictationError> {
        let stopped_at = Instant::now();
        if let Some(pass) = self.pass.take() {
            pass.abort();
        }
        let end_ms = pcm.len() as u64 / SAMPLES_PER_MS;
        let tail = self.mark_ms..end_ms;
        if let Some(jobs) = self.jobs.take() {
            if !tail.is_empty() {
                let _ = jobs.send(Job::Tail {
                    pcm: copy_ms(pcm, &tail),
                    offset_ms: tail.start,
                    open_speech_ms: self.open_speech_ms,
                    stopped_at,
                });
            }
        }
        match (&mut self.decoder).await {
            Ok(decoded) => decoded,
            Err(_) => Err(DictationError::Internal(
                "the dictation decoder stopped".into(),
            )),
        }
    }
}

impl Drop for LiveDecode {
    fn drop(&mut self) {
        if let Some(pass) = self.pass.take() {
            pass.abort();
        }
        self.decoder.abort();
    }
}

/// The decoder: every job in order on the shared worker. A failure ends it.
async fn decode_jobs(
    engine: Arc<dyn Engine>,
    request: impl std::future::Future<Output = TranscribeRequest> + Send,
    mut jobs: mpsc::UnboundedReceiver<Job>,
) -> Result<Decoded, DictationError> {
    fn add(out: &mut Decoded, offset_ms: u64, length_ms: u64, segment: SegmentDecode) {
        out.segments += 1;
        out.encode_ms += segment.encode_ms;
        out.decode_ms += segment.decode_ms;
        if out.retried.is_none() {
            out.retried = segment.retried;
        }
        out.pieces.extend(
            segment
                .pieces
                .iter()
                .map(|p| place(offset_ms, length_ms, p.t0_ms, p.t1_ms, &p.text)),
        );
    }
    let request = request.await;
    let mut out = Decoded::default();
    while let Some(job) = jobs.recv().await {
        match job {
            Job::Segment {
                pcm,
                offset_ms,
                speech_ms,
            } => {
                let length_ms = pcm.len() as u64 / SAMPLES_PER_MS;
                let segment = engine
                    .decode(pcm.slice(), Some(speech_ms), &request)
                    .await?;
                drop(pcm);
                add(&mut out, offset_ms, length_ms, segment);
                out.segments_while_speaking += 1;
            }
            Job::Tail {
                pcm,
                offset_ms,
                open_speech_ms,
                stopped_at,
            } => {
                let audio_ms = pcm.len() as u64 / SAMPLES_PER_MS;
                out.tail_audio_s = audio_ms as f64 / 1_000.0;
                if pcm.0.iter().all(|&s| s == 0) {
                    continue;
                }
                let whole = pcm.slice();
                if audio_ms <= MAX_SEGMENT_MS {
                    let speech = (open_speech_ms > 0).then_some(open_speech_ms);
                    let segment = engine.decode(whole, speech, &request).await?;
                    add(&mut out, offset_ms, audio_ms, segment);
                } else {
                    let spans = engine.vad(whole.clone()).await?;
                    for range in
                        biorouter::dictation::file::pack(plan_final_segments(&spans, 0, audio_ms))
                    {
                        let segment = engine
                            .decode(
                                whole.slice_ms(&range),
                                Some(speech_within(&spans, &range)),
                                &request,
                            )
                            .await?;
                        add(
                            &mut out,
                            offset_ms + range.start,
                            range.end - range.start,
                            segment,
                        );
                    }
                }
                out.tail_ms = stopped_at.elapsed().as_millis() as u64;
            }
        }
    }
    Ok(out)
}

/// The window the next detector pass reads, when one is due: 8 s past the mark, at most
/// 60 s, and only past where a pass that moved nothing last stopped.
fn due_window(mark_ms: u64, end_ms: u64, stalled_at_ms: Option<u64>) -> Option<Range<u64>> {
    if end_ms < mark_ms.saturating_add(STREAM_VAD_TRIGGER_MS) {
        return None;
    }
    let window = mark_ms..end_ms.min(mark_ms.saturating_add(MAX_DETECTOR_PASS_MS));
    if stalled_at_ms.is_some_and(|at| window.end <= at) {
        return None;
    }
    Some(window)
}

/// One pass over `[mark, window_end)`: `plan_closed_segments`, plus the daemon's two rules
/// so every full pass moves the mark: a window with no speech moves it to 1 s before its
/// end, and speech with no closing pause is cut every 25 s.
fn plan_pass(spans: &[Span], mark: u64, window_end: u64) -> (Vec<Range<u64>>, u64) {
    let (closed, next) = plan_closed_segments(spans, mark, window_end);
    if !closed.is_empty() || window_end < mark.saturating_add(STREAM_VAD_TRIGGER_MS) {
        return (closed, next.max(mark));
    }
    if !spans.iter().any(|s| s.t1_ms > mark && s.t0_ms < window_end) {
        return (
            Vec::new(),
            window_end.saturating_sub(SILENCE_KEPT_MS).max(mark),
        );
    }
    if window_end >= mark + MAX_SEGMENT_MS + STREAM_VAD_TRIGGER_MS {
        let cut = mark + MAX_SEGMENT_MS;
        return (plan_final_segments(spans, mark, cut), cut);
    }
    (Vec::new(), mark)
}

fn speech_within(spans: &[Span], range: &Range<u64>) -> u64 {
    spans
        .iter()
        .map(|s| {
            s.t1_ms
                .min(range.end)
                .saturating_sub(s.t0_ms.max(range.start))
        })
        .sum()
}

fn copy_ms(pcm: &[i16], range: &Range<u64>) -> AudioCopy {
    let at = |ms: u64| usize::try_from(ms.saturating_mul(SAMPLES_PER_MS)).unwrap_or(usize::MAX);
    let end = at(range.end).min(pcm.len());
    let start = at(range.start).min(end);
    AudioCopy(pcm[start..end].into())
}

fn place(offset_ms: u64, length_ms: u64, t0_ms: u64, t1_ms: u64, text: &str) -> Piece {
    let t1_ms = (offset_ms + t1_ms).min(offset_ms + length_ms);
    let t0_ms = (offset_ms + t0_ms).min(t1_ms);
    Piece {
        t0_ms,
        t1_ms,
        text: text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn second(value: f32) -> Vec<f32> {
        vec![value; 16_000]
    }

    /// Exact zeros for 1.5 s are the permission path, and end the recording.
    #[test]
    fn exact_zeros_for_a_second_and_a_half_are_the_denial() {
        let mut recorder = Recorder::new(None);
        assert!(recorder.push(&second(0.0)).is_empty());
        assert_eq!(recorder.push(&second(0.0)), vec![Notice::Denied]);
        assert!(
            recorder.push(&second(0.3)).is_empty(),
            "nothing after the end"
        );
        // One sample that is not zero in that window is a working microphone.
        let mut working = Recorder::new(None);
        let mut first = second(0.0);
        first[100] = 1e-6;
        working.push(&first);
        assert!(!working.push(&second(0.0)).contains(&Notice::Denied));
    }

    #[test]
    fn under_half_a_second_is_too_short() {
        let mut recorder = Recorder::new(None);
        recorder.push(&vec![0.2; 7_000]);
        assert_eq!(recorder.finish().unwrap_err(), Ended::TooShort);
        let mut long_enough = Recorder::new(None);
        long_enough.push(&vec![0.2; 8_000]);
        assert_eq!(long_enough.finish().unwrap().len(), 8_000);
        let mut zeros = Recorder::new(None);
        zeros.push(&vec![0.0; 12_000]);
        assert_eq!(zeros.finish().unwrap_err(), Ended::Silent);
    }

    /// Below the noise floor for 3 s, before anything was heard: MSG-NO-AUDIO-YET once, and
    /// the recording goes on. A pause after speech is not that.
    #[test]
    fn a_quiet_microphone_is_told_once_and_keeps_recording() {
        let mut recorder = Recorder::new(None);
        let quiet = second(0.001);
        let mut notices = Vec::new();
        for _ in 0..5 {
            notices.extend(recorder.push(&quiet));
        }
        assert_eq!(notices, vec![Notice::NoAudioYet]);
        assert_eq!(recorder.duration_ms(), 5_000);
        let mut talker = Recorder::new(None);
        talker.push(&second(0.2));
        for _ in 0..5 {
            assert!(talker.push(&quiet).is_empty());
        }
    }

    /// The 600 s cap: MSG-CAP-WARN at 570 s, the stop at 600 s, and nothing kept past it.
    #[test]
    fn the_ten_minute_cap() {
        let mut recorder = Recorder::new(None);
        let mut notices = Vec::new();
        for _ in 0..605 {
            notices.extend(recorder.push(&second(0.1)));
        }
        assert_eq!(
            notices,
            vec![Notice::CapWarn, Notice::Stop(StopReason::Cap)]
        );
        assert_eq!(recorder.duration_ms(), MAX_RECORDING_MS);
    }

    #[test]
    fn seconds_stop_sooner_and_never_later() {
        let mut recorder = Recorder::new(Some(3));
        let mut notices = Vec::new();
        for _ in 0..5 {
            notices.extend(recorder.push(&second(0.1)));
        }
        assert_eq!(notices, vec![Notice::Stop(StopReason::Seconds)]);
        assert_eq!(recorder.duration_ms(), 3_000);
        // At 600 or more it is the cap itself.
        let cap = Recorder::new(Some(600));
        assert_eq!(cap.limit_reason, StopReason::Cap);
    }

    /// Three failures within 10 s pause dictation for the session; failures further apart
    /// do not.
    #[test]
    fn three_failures_within_ten_seconds_pause() {
        let start = Instant::now();
        let mut failures = Failures::default();
        assert!(!failures.record(start));
        assert!(!failures.record(start + Duration::from_secs(4)));
        assert!(failures.record(start + Duration::from_secs(9)));
        assert!(failures.paused());
        let mut apart = Failures::default();
        for i in 0..5 {
            assert!(!apart.record(start + Duration::from_secs(i * 6)));
        }
    }

    #[test]
    fn a_failed_recording_is_kept_for_five_minutes() {
        let start = Instant::now();
        let mut hold = RetryHold::default();
        hold.keep(vec![1, 2, 3], start);
        assert_eq!(
            hold.take(start + Duration::from_secs(299)),
            Some(vec![1, 2, 3])
        );
        assert_eq!(hold.take(start), None, "taken once");
        hold.keep(vec![4], start);
        assert_eq!(hold.take(start + Duration::from_secs(301)), None);
        hold.keep(vec![5], start);
        hold.clear();
        assert_eq!(hold.take(start), None);
    }

    /// An engine that records what it was asked and answers each decode with one piece
    /// naming the audio's length.
    #[derive(Default)]
    struct FakeEngine {
        calls: Mutex<Vec<String>>,
        /// The detector's spans, in the window's own milliseconds.
        spans: Mutex<Vec<Span>>,
        fail_decode: bool,
    }

    #[async_trait::async_trait]
    impl Engine for FakeEngine {
        async fn vad(&self, pcm: PcmSlice) -> Result<Vec<Span>, DictationError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("vad {}", pcm.duration_ms()));
            Ok(self.spans.lock().unwrap().clone())
        }

        async fn decode(
            &self,
            pcm: PcmSlice,
            speech_ms: Option<u64>,
            _request: &TranscribeRequest,
        ) -> Result<SegmentDecode, DictationError> {
            self.calls.lock().unwrap().push(format!(
                "decode {} {:?}",
                pcm.duration_ms(),
                speech_ms
            ));
            if self.fail_decode {
                return Err(DictationError::Cancelled);
            }
            Ok(SegmentDecode {
                pieces: vec![biorouter_dictation::protocol::SegmentText {
                    t0_ms: 0,
                    t1_ms: pcm.duration_ms(),
                    text: format!("heard {} ms", pcm.duration_ms()),
                }],
                ..SegmentDecode::default()
            })
        }
    }

    async fn request() -> TranscribeRequest {
        TranscribeRequest::new("en")
    }

    /// A short dictation is all tail: one decode at stop, no detector pass.
    #[tokio::test]
    async fn a_short_dictation_is_one_decode_at_stop() {
        let engine = Arc::new(FakeEngine::default());
        let mut live = LiveDecode::new(engine.clone(), request());
        let pcm = vec![100i16; 16_000 * 5];
        live.poll(&pcm);
        let decoded = live.finish(&pcm).await.unwrap();
        assert_eq!(*engine.calls.lock().unwrap(), vec!["decode 5000 None"]);
        assert_eq!(decoded.segments, 1);
        assert_eq!(decoded.segments_while_speaking, 0);
        assert_eq!(decoded.raw_text(), "heard 5000 ms");
        assert_eq!(decoded.tail_audio_s, 5.0);
    }

    /// Closed speech is decoded while the person speaks: once 8 s lie past the mark, the
    /// detector reads them and the closed stretch is queued, so the stop decodes only the
    /// open tail after the mark.
    #[tokio::test]
    async fn closed_segments_are_decoded_while_speaking() {
        let engine = Arc::new(FakeEngine::default());
        // Speech from 0.5 s to 3 s, a pause, then speech from 6 s that is still open.
        *engine.spans.lock().unwrap() = vec![
            Span {
                t0_ms: 500,
                t1_ms: 3_000,
            },
            Span {
                t0_ms: 6_000,
                t1_ms: 8_400,
            },
        ];
        let mut live = LiveDecode::new(engine.clone(), request());
        let mut pcm = vec![100i16; 16_000 * 7];
        live.poll(&pcm);
        assert!(live.pass.is_none(), "no pass before 8 s");
        pcm.extend(vec![100i16; 16_000 * 2 - 6_400]);
        live.poll(&pcm);
        assert!(live.pass.is_some(), "a pass at 8.6 s");
        for _ in 0..200 {
            if live.pass.as_ref().is_some_and(JoinHandle::is_finished) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        live.poll(&pcm);
        assert!(
            live.mark_ms() > 3_000 && live.mark_ms() < 6_000,
            "{}",
            live.mark_ms()
        );
        let mark = live.mark_ms();
        let decoded = live.finish(&pcm).await.unwrap();
        let calls = engine.calls.lock().unwrap().clone();
        assert_eq!(calls[0], "vad 8600");
        assert!(calls[1].starts_with("decode "), "{calls:?}");
        assert_eq!(decoded.segments_while_speaking, 1);
        assert_eq!(decoded.segments, 2);
        let tail_ms = 8_600 - mark;
        assert_eq!(
            calls.last().unwrap(),
            &format!("decode {tail_ms} Some(2400)")
        );
        // The pieces keep the recording's own times, in order.
        assert!(decoded.pieces[0].t1_ms <= decoded.pieces[1].t0_ms);
    }

    #[tokio::test]
    async fn a_failed_decode_fails_the_recording() {
        let engine = Arc::new(FakeEngine {
            fail_decode: true,
            ..FakeEngine::default()
        });
        let live = LiveDecode::new(engine, request());
        let pcm = vec![100i16; 16_000];
        assert!(live.finish(&pcm).await.is_err());
    }

    /// The debug build's stand in engine answers the tail with its text, so the review and
    /// the clipboard paths run end to end without a speech model.
    #[tokio::test]
    async fn the_fixed_text_engine_answers_the_tail() {
        let live = LiveDecode::new(
            Arc::new(FixedText {
                text: "EPAS1 levels rose".into(),
            }),
            request(),
        );
        let pcm = vec![100i16; 16_000 * 2];
        let decoded = live.finish(&pcm).await.unwrap();
        assert_eq!(decoded.raw_text(), "EPAS1 levels rose");
        assert_eq!(decoded.pieces[0].t1_ms, 2_000);
    }

    #[tokio::test]
    async fn a_silent_tail_costs_no_decode() {
        let engine = Arc::new(FakeEngine::default());
        let live = LiveDecode::new(engine.clone(), request());
        let pcm = vec![0i16; 16_000 * 2];
        let decoded = live.finish(&pcm).await.unwrap();
        assert!(engine.calls.lock().unwrap().is_empty());
        assert_eq!(decoded.raw_text(), "");
    }

    /// A tail over 25 s (speech that never paused) is read by the detector and planned like
    /// a recording.
    #[tokio::test]
    async fn a_long_tail_is_planned_like_a_recording() {
        let engine = Arc::new(FakeEngine::default());
        *engine.spans.lock().unwrap() = vec![
            Span {
                t0_ms: 0,
                t1_ms: 14_000,
            },
            Span {
                t0_ms: 15_000,
                t1_ms: 29_000,
            },
        ];
        let live = LiveDecode::new(engine.clone(), request());
        let pcm = vec![100i16; 16_000 * 30];
        let decoded = live.finish(&pcm).await.unwrap();
        let calls = engine.calls.lock().unwrap().clone();
        assert_eq!(calls[0], "vad 30000");
        assert!(decoded.segments >= 2, "{calls:?}");
    }

    #[test]
    fn a_pass_over_silence_moves_the_mark() {
        let (closed, mark) = plan_pass(&[], 0, 9_000);
        assert!(closed.is_empty());
        assert_eq!(mark, 8_000);
        assert_eq!(due_window(0, 7_999, None), None);
        assert_eq!(due_window(0, 9_000, None), Some(0..9_000));
        assert_eq!(due_window(0, 9_000, Some(9_000)), None);
        assert_eq!(due_window(0, 90_000, None), Some(0..60_000));
    }
}
