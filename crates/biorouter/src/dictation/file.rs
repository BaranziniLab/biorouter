//! A whole recording through one loaded model (PR-12; DD-47, DD-48, DD-50):
//! `biorouter transcribe`, and any later caller that holds a finished recording.
//!
//! The recording arrives as 16 kHz mono samples; the CLI decodes files, and the engine
//! and the routes take PCM only (DD-78). The voice detector runs over it in windows of at
//! most 600 s, the most one `vad_segments` takes (DD-28). Each window's closed stretches of
//! speech are planned by the planner a live dictation uses
//! ([`audio::plan_closed_segments`], [`audio::plan_final_segments`]): segments of at most
//! 25 s, split at the longest pause, with 0.3 s kept before the first and after the last
//! stretch of each. A segment is queued the moment it is planned, before the next window
//! is detected, so the worker decodes while the detector reads on; the supervisor runs
//! detector jobs before queued decodes, so the next window never waits behind them. The
//! model is loaded once for the whole recording.
//!
//! What each decode returns goes through the recognizer ([`recognize`]): a piece that is
//! only tags is dropped, the echo and collapse checks run against the prompt that was
//! sent, the pieces are joined in order, and the whole output filter empties an output
//! with no speech in it. A recording of exact zeros, or one in which the detector finds no
//! speech, is answered `no_speech` without a decode.
//!
//! [`recognize`]: biorouter_dictation::recognize

use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use biorouter_dictation::audio::{
    plan_closed_segments, plan_final_segments, PcmSlice, Span, MAX_SEGMENT_MS, SAMPLES_PER_MS,
};
use biorouter_dictation::protocol::{SegmentText, TranscribeResult, VadOptions};
use biorouter_dictation::recognize::{echo, text, RetryReason};
use biorouter_dictation::supervisor::{
    JobId, LoadSpec, StreamKey, Supervisor, SupervisorError, TranscribeParams,
};
use serde::Serialize;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::service::DictationError;

/// Under this much audio nothing is transcribed (MSG-TOO-SHORT).
pub const MIN_AUDIO_MS: u64 = 500;
/// The longest recording `transcribe` takes (DD-78).
pub const MAX_AUDIO_MS: u64 = 2 * 60 * 60 * 1_000;
/// One detector request: the 600 s cap of `vad_segments` (DD-28).
pub const VAD_WINDOW_MS: u64 = 600_000;
/// When a window holds no speech the next starts this long before its end, so speech
/// that begins in a window's last moments is detected whole.
pub const WINDOW_OVERLAP_MS: u64 = 1_000;

/// A piece of the transcript, in milliseconds from the start of the recording.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TimedText {
    pub t0_ms: u64,
    pub t1_ms: u64,
    pub text: String,
}

/// Where the time went, for `transcribe --format json` and the accuracy test.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RecordingTimings {
    /// Every detector request, from its submission to its answer.
    pub vad_ms: u64,
    /// The worker's own encoder and decoder time, summed over every decode.
    pub encode_ms: u64,
    pub decode_ms: u64,
    /// How many `transcribe` requests ran, retries included.
    pub decodes: u32,
    /// From the first detector request to the last answer.
    pub wall_ms: u64,
}

/// A recording's transcript, before correction (PR-14 adds the dictionary's).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RecordingTranscript {
    /// The final text. Equal to `raw_text` until PR-14 corrects it.
    pub text: String,
    /// As the engine wrote it, after the nonspeech filter.
    pub raw_text: String,
    pub segments: Vec<TimedText>,
    pub no_speech: bool,
    /// The first reason a decode was retried, if one was.
    pub retried: Option<RetryReason>,
    pub audio_ms: u64,
    /// The detector's speech, overlaps counted once.
    pub speech_ms: u64,
    pub timings: RecordingTimings,
}

impl RecordingTranscript {
    fn nothing_heard(audio_ms: u64, speech_ms: u64, timings: RecordingTimings) -> Self {
        RecordingTranscript {
            text: String::new(),
            raw_text: String::new(),
            segments: Vec::new(),
            no_speech: true,
            retried: None,
            audio_ms,
            speech_ms,
            timings,
        }
    }
}

/// Where the detector looks next, and what each window's spans let the decoder start
/// on. Pure, so its coverage and its progress are tested without a worker.
#[derive(Clone, Debug)]
pub struct WindowPlanner {
    mark: u64,
    end: u64,
    done: bool,
}

impl WindowPlanner {
    /// A recording of `audio_ms`.
    pub fn new(audio_ms: u64) -> WindowPlanner {
        WindowPlanner {
            mark: 0,
            end: audio_ms,
            done: false,
        }
    }

    /// The next window to detect, `None` once the recording is planned to its end.
    pub fn next_window(&self) -> Option<Range<u64>> {
        (!self.done).then(|| self.mark..self.mark.saturating_add(VAD_WINDOW_MS).min(self.end))
    }

    /// The segments that `spans` (the detector's answer for `window`, in milliseconds of
    /// the recording) close, and the mark moved past them. The last window plans
    /// everything. Any other window plans what it closed; when it closed nothing it moves
    /// on anyway: past a window with no speech (keeping a second of overlap), or, under
    /// speech that never pauses, up to 25 s before the window's end, which is cut every 25
    /// s and detected again from there. What is planned is then [`pack`]ed.
    pub fn accept(&mut self, window: &Range<u64>, spans: &[Span]) -> Vec<Range<u64>> {
        pack(self.plan(window, spans))
    }

    fn plan(&mut self, window: &Range<u64>, spans: &[Span]) -> Vec<Range<u64>> {
        if window.end >= self.end {
            self.done = true;
            return plan_final_segments(spans, self.mark, self.end);
        }
        let (segments, mark) = plan_closed_segments(spans, self.mark, window.end);
        if mark > self.mark {
            self.mark = mark;
            return segments;
        }
        let heard = spans
            .iter()
            .any(|s| s.t1_ms > self.mark && s.t0_ms < window.end);
        if !heard {
            self.mark = window
                .end
                .saturating_sub(WINDOW_OVERLAP_MS)
                .max(self.mark + 1);
            return Vec::new();
        }
        let cut = window.end.saturating_sub(MAX_SEGMENT_MS).max(self.mark + 1);
        let segments = plan_final_segments(spans, self.mark, cut);
        self.mark = cut;
        segments
    }
}

/// Consecutive segments joined, from the first, while the joined one stays within 25 s.
///
/// The planner splits a long stretch at its longest pause, level by level. Where the
/// pauses are alike that peels one sentence off per level: 13.6 minutes of sentences 2 s
/// apart became 60 decodes, each paying a whole encoder pass, which was 14.5 s of turbo's
/// 21 s on Metal [M]. Joined, every cut that remains is one the planner made at a pause,
/// and no decode passes 25 s.
pub fn pack(segments: Vec<Range<u64>>) -> Vec<Range<u64>> {
    let mut packed: Vec<Range<u64>> = Vec::with_capacity(segments.len());
    for segment in segments {
        match packed.last_mut() {
            Some(last)
                if segment.start >= last.end
                    && segment.end.saturating_sub(last.start) <= MAX_SEGMENT_MS =>
            {
                last.end = segment.end;
            }
            _ => packed.push(segment),
        }
    }
    packed
}

/// Stream keys for recordings: a range of their own, two per recording (the decodes,
/// then their retries, which go round robin beside them instead of behind them).
fn next_stream() -> StreamKey {
    static NEXT: AtomicU64 = AtomicU64::new(1 << 60);
    NEXT.fetch_add(2, Ordering::Relaxed)
}

/// The detector's speech inside `range`, in milliseconds.
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

/// The milliseconds covered by `spans`, overlaps counted once.
fn union_ms(spans: &[Span]) -> u64 {
    let mut sorted: Vec<Span> = spans.to_vec();
    sorted.sort_by_key(|s| s.t0_ms);
    let (mut total, mut reach) = (0u64, 0u64);
    for span in sorted {
        let start = span.t0_ms.max(reach);
        if span.t1_ms > start {
            total += span.t1_ms - start;
        }
        reach = reach.max(span.t1_ms);
    }
    total
}

type Answer = oneshot::Receiver<Result<TranscribeResult, SupervisorError>>;

struct Queued {
    range: Range<u64>,
    speech_ms: u64,
    answer: Answer,
}

/// Transcribes `pcm` (16 kHz mono) with the model `spec` names, through `supervisor`.
/// `params` carries the language, the decode and detector values, and the prompt (empty
/// until PR-14). Cancelling `cancel` answers `Cancelled` at once and drops or aborts every
/// job the recording queued; so does a failed job, so the worker does not go on decoding a
/// recording whose transcript can no longer be given.
pub async fn transcribe_recording(
    supervisor: &Supervisor,
    spec: Arc<LoadSpec>,
    pcm: Arc<[i16]>,
    params: TranscribeParams,
    cancel: &CancellationToken,
) -> Result<RecordingTranscript, DictationError> {
    let stream = next_stream();
    let detecting: Mutex<Option<JobId>> = Mutex::new(None);
    let work = run(supervisor, spec, pcm, params, stream, &detecting);
    tokio::select! {
        result = work => {
            if result.is_err() {
                supervisor.cancel_stream(stream);
                supervisor.cancel_stream(stream + 1);
            }
            result
        }
        () = cancel.cancelled() => {
            supervisor.cancel_stream(stream);
            supervisor.cancel_stream(stream + 1);
            if let Some(job) = *detecting.lock().unwrap_or_else(PoisonError::into_inner) {
                supervisor.cancel(job);
            }
            Err(DictationError::Cancelled)
        }
    }
}

async fn run(
    supervisor: &Supervisor,
    spec: Arc<LoadSpec>,
    pcm: Arc<[i16]>,
    params: TranscribeParams,
    stream: StreamKey,
    detecting: &Mutex<Option<JobId>>,
) -> Result<RecordingTranscript, DictationError> {
    let whole = PcmSlice::whole(Arc::clone(&pcm));
    let audio_ms = whole.duration_ms();
    let mut timings = RecordingTimings::default();
    // DD-48: an all zero buffer is no speech, and costs no request at all.
    if pcm.iter().all(|&s| s == 0) {
        return Ok(RecordingTranscript::nothing_heard(audio_ms, 0, timings));
    }
    let started = Instant::now();
    let mut planner = WindowPlanner::new(audio_ms);
    let mut heard: Vec<Span> = Vec::new();
    let mut queued: Vec<Queued> = Vec::new();
    while let Some(window) = planner.next_window() {
        let asked = Instant::now();
        let (job, answer) = supervisor.submit_vad_segments(
            Arc::clone(&spec),
            whole.slice_ms(&window),
            VadOptions::dd48(),
        );
        *detecting.lock().unwrap_or_else(PoisonError::into_inner) = Some(job);
        let found = answer.await.unwrap_or(Err(SupervisorError::Stopped));
        *detecting.lock().unwrap_or_else(PoisonError::into_inner) = None;
        timings.vad_ms += asked.elapsed().as_millis() as u64;
        let spans: Vec<Span> = found?
            .into_iter()
            .map(|s| Span {
                t0_ms: s.t0_ms + window.start,
                t1_ms: (s.t1_ms + window.start).min(window.end),
            })
            .filter(|s| s.t0_ms < s.t1_ms)
            .collect();
        for range in planner.accept(&window, &spans) {
            let (_, answer) = supervisor.submit_transcribe(
                Arc::clone(&spec),
                stream,
                whole.slice_ms(&range),
                params.clone(),
            );
            queued.push(Queued {
                speech_ms: speech_within(&spans, &range),
                range,
                answer,
            });
        }
        heard.extend(spans);
    }
    let speech_ms = union_ms(&heard);
    if queued.is_empty() {
        timings.wall_ms = started.elapsed().as_millis() as u64;
        return Ok(RecordingTranscript::nothing_heard(
            audio_ms, speech_ms, timings,
        ));
    }

    let prompt_sent = !params.prompt_terms.is_empty();
    let mut retried = None;
    let mut pieces: Vec<TimedText> = Vec::new();
    for job in queued {
        let result = job.answer.await.unwrap_or(Err(SupervisorError::Stopped))?;
        count(&mut timings, &result);
        let sent = sent_prompt(&params.prompt_terms, result.prompt_terms_used);
        let assessment = echo::assess(&result.text, sent, job.speech_ms);
        let mut kept = Some(result);
        if let Some(reason) = assessment
            .retry
            .filter(|_| echo::retry_changes_anything(prompt_sent))
        {
            retried.get_or_insert(reason);
            let unprompted = TranscribeParams {
                prompt_terms: Vec::new(),
                ..params.clone()
            };
            let (_, answer) = supervisor.submit_transcribe(
                Arc::clone(&spec),
                stream + 1,
                whole.slice_ms(&job.range),
                unprompted,
            );
            let again = answer.await.unwrap_or(Err(SupervisorError::Stopped))?;
            count(&mut timings, &again);
            // Judged against the prompt that was sent, as the first decode was: a term the
            // worker could not fit was never in the decoder's context, so the retry cannot
            // have recited it.
            kept = echo::adopt_retry(&again.text, sent).then_some(again);
        } else if assessment.recital_stripped {
            // The recital is the last sentence; the pieces keep their times, so the
            // stripped text replaces them as one piece.
            if let Some(result) = kept.as_mut() {
                result.segments = vec![biorouter_dictation::protocol::SegmentText {
                    t0_ms: result.segments.first().map_or(0, |s| s.t0_ms),
                    t1_ms: result.segments.last().map_or(0, |s| s.t1_ms),
                    text: assessment.text.clone(),
                }];
            }
        }
        let Some(result) = kept else {
            continue;
        };
        for piece in result.segments {
            if !text::piece_has_words(&piece.text) {
                continue;
            }
            let (t0_ms, t1_ms) = place(&job.range, &piece);
            pieces.push(TimedText {
                t0_ms,
                t1_ms,
                text: piece.text.trim().to_string(),
            });
        }
    }
    timings.wall_ms = started.elapsed().as_millis() as u64;
    let raw_text = text::filter_whole_output(&text::join(pieces.iter().map(|p| p.text.as_str())));
    if raw_text.is_empty() {
        let mut nothing = RecordingTranscript::nothing_heard(audio_ms, speech_ms, timings);
        nothing.retried = retried;
        return Ok(nothing);
    }
    Ok(RecordingTranscript {
        text: raw_text.clone(),
        raw_text,
        segments: pieces,
        no_speech: false,
        retried,
        audio_ms,
        speech_ms,
        timings,
    })
}

/// A decoded piece's times in the recording: its times in the decode, moved to where the
/// segment starts, and kept inside the segment with the start never after the end. Whisper
/// can time a piece past the audio it was given (a trailing invention that is not a tag),
/// and an SRT or WebVTT cue that ends before it starts is invalid.
fn place(range: &Range<u64>, piece: &SegmentText) -> (u64, u64) {
    let t1_ms = (range.start + piece.t1_ms).min(range.end);
    let t0_ms = (range.start + piece.t0_ms).min(t1_ms);
    (t0_ms, t1_ms)
}

/// The terms the worker fitted into the token budget: the last `used` of `terms`.
fn sent_prompt(terms: &[String], used: u32) -> &[String] {
    let used = (used as usize).min(terms.len());
    terms.get(terms.len() - used..).unwrap_or_default()
}

fn count(timings: &mut RecordingTimings, result: &TranscribeResult) {
    timings.encode_ms += result.timings.encode_ms.round() as u64;
    timings.decode_ms += result.timings.decode_ms.round() as u64;
    timings.decodes += 1;
}

/// The recording's length in milliseconds, rounded up.
pub fn duration_ms(samples: usize) -> u64 {
    (samples as u64).div_ceil(SAMPLES_PER_MS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter_dictation::cpu;
    use biorouter_dictation::protocol::{
        self, read_frame, write_frame, ErrorCode, ErrorMsg, FromWorker, Hello, Loaded, SegmentText,
        Segments, Timings, ToWorker, PROTOCOL_VERSION,
    };
    use biorouter_dictation::supervisor::{
        SupervisorConfig, WorkerChild, WorkerProcess, WorkerSpawner,
    };
    use std::future::Future;
    use std::io;
    use std::pin::Pin;
    use std::time::Duration;
    use tokio::io::DuplexStream;

    fn span(t0_ms: u64, t1_ms: u64) -> Span {
        Span { t0_ms, t1_ms }
    }

    /// The detector's answer for `window`, from the speech the recording really holds.
    fn detect(speech: &[Span], window: &Range<u64>) -> Vec<Span> {
        speech
            .iter()
            .filter(|s| s.t1_ms > window.start && s.t0_ms < window.end)
            .map(|s| span(s.t0_ms.max(window.start), s.t1_ms.min(window.end)))
            .collect()
    }

    /// Every window the planner asks for, and every segment it plans.
    fn plan(audio_ms: u64, speech: &[Span]) -> (Vec<Range<u64>>, Vec<Range<u64>>) {
        let mut planner = WindowPlanner::new(audio_ms);
        let (mut windows, mut segments) = (Vec::new(), Vec::new());
        while let Some(window) = planner.next_window() {
            assert!(windows.len() < 100, "the planner does not move on");
            let spans = detect(speech, &window);
            segments.extend(planner.accept(&window, &spans));
            windows.push(window);
        }
        (windows, segments)
    }

    fn assert_plan_holds(audio_ms: u64, speech: &[Span], segments: &[Range<u64>]) {
        for pair in segments.windows(2) {
            assert!(
                pair[0].end <= pair[1].start,
                "overlapping segments: {pair:?}"
            );
        }
        for segment in segments {
            assert!(segment.start < segment.end && segment.end <= audio_ms);
            assert!(segment.end - segment.start <= MAX_SEGMENT_MS, "{segment:?}");
        }
        // Every millisecond of speech is inside a decoded segment.
        for s in speech {
            let covered: u64 = segments
                .iter()
                .map(|r| s.t1_ms.min(r.end).saturating_sub(s.t0_ms.max(r.start)))
                .sum();
            assert_eq!(
                covered,
                s.t1_ms - s.t0_ms,
                "{s:?} is not covered: {segments:?}"
            );
        }
    }

    #[test]
    fn a_recording_under_one_window_is_planned_once() {
        let speech = [
            span(500, 12_000),
            span(13_500, 26_000),
            span(27_000, 39_500),
        ];
        let (windows, segments) = plan(40_000, &speech);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0], 0..40_000);
        assert_eq!(segments.len(), 3, "{segments:?}");
        assert_plan_holds(40_000, &speech, &segments);
        // Split at the pauses, never inside speech.
        assert!(segments[0].end <= 13_500 && segments[1].start >= 12_000);
        assert!(segments[1].end <= 27_000 && segments[2].start >= 26_000);
        // DD-48: 0.3 s kept before the first and after the last stretch of speech.
        assert_eq!(segments[0].start, 500 - 300);
        assert_eq!(segments[2].end, 39_500 + 300);
    }

    #[test]
    fn a_long_recording_is_planned_window_by_window_and_covered() {
        // 8 s of speech, 2 s of pause, for 1,300 s: speech crosses both window edges.
        let speech: Vec<Span> = (0..130)
            .map(|i| span(i * 10_000 + 1_000, i * 10_000 + 9_000))
            .collect();
        let (windows, segments) = plan(1_300_000, &speech);
        assert!(windows.len() >= 3, "{windows:?}");
        assert!(windows.iter().all(|w| w.end - w.start <= VAD_WINDOW_MS));
        assert_plan_holds(1_300_000, &speech, &segments);
    }

    #[test]
    fn speech_that_never_pauses_is_cut_every_25_s_across_windows() {
        let speech = [span(0, 1_300_000)];
        let (windows, segments) = plan(1_300_000, &speech);
        assert!(windows.len() >= 3);
        assert_plan_holds(1_300_000, &speech, &segments);
        assert!(segments.len() >= 52, "{}", segments.len());
    }

    #[test]
    fn silence_is_passed_over_and_a_late_word_is_found() {
        let (windows, segments) = plan(1_800_000, &[]);
        assert!(segments.is_empty());
        assert!(windows.len() >= 3);
        // Speech that starts just before a window's end is decoded whole.
        let speech = [span(599_600, 601_500)];
        let (_, segments) = plan(1_800_000, &speech);
        assert_plan_holds(1_800_000, &speech, &segments);
        assert_eq!(segments.len(), 1, "{segments:?}");
    }

    /// Evenly paced speech is decoded in segments near 25 s, not a sentence at a time.
    #[test]
    fn evenly_paced_speech_is_packed_near_25_s() {
        // 5 s sentences 1 s apart for 120 s.
        let speech: Vec<Span> = (0..20)
            .map(|i| span(i * 6_000 + 500, i * 6_000 + 5_500))
            .collect();
        let (_, segments) = plan(120_000, &speech);
        assert_plan_holds(120_000, &speech, &segments);
        assert!(
            segments.len() <= 6,
            "{} segments: {segments:?}",
            segments.len()
        );
        assert_eq!(
            pack(vec![
                0..10_000,
                10_500..20_000,
                20_500..26_000,
                26_000..30_000
            ]),
            vec![0..20_000, 20_500..30_000]
        );
        assert!(pack(Vec::new()).is_empty());
    }

    #[test]
    fn spans_and_sums() {
        assert_eq!(union_ms(&[span(0, 10), span(5, 20), span(30, 40)]), 30);
        assert_eq!(speech_within(&[span(0, 10), span(20, 40)], &(5..30)), 15);
        let terms: Vec<String> = ["a", "b", "c"].iter().map(|t| t.to_string()).collect();
        assert_eq!(sent_prompt(&terms, 2), &terms[1..]);
        assert_eq!(sent_prompt(&terms, 9), &terms[..]);
        assert!(sent_prompt(&terms, 0).is_empty());
        assert_eq!(duration_ms(16_001), 1_001);
    }

    // ---- The pipeline against an in-memory worker -------------------------------------

    /// What the in-memory worker received.
    #[derive(Default)]
    struct Seen {
        spawns: usize,
        /// Per `transcribe`: its samples and `no_context`.
        decodes: Vec<(usize, bool)>,
        vads: usize,
        /// Answer this `transcribe` (counted from 1) with an error.
        fail_decode: Option<usize>,
        /// How long each `transcribe` takes.
        decode_time: Duration,
        /// Per `transcribe`: how many prompt terms it carried.
        prompts: Vec<usize>,
        /// When set, a prompted decode answers this text, and an unprompted one the other.
        prompted_text: Option<String>,
        unprompted_text: Option<String>,
        /// How many prompt terms the worker fits into the budget; all of them when `None`.
        fit: Option<usize>,
    }

    /// Speech is any 10 ms block that is not all zeros; the in-memory detector reports
    /// such blocks as spans, the way Silero would report a tone.
    fn loud_spans(pcm: &[i16]) -> Vec<Span> {
        let mut spans: Vec<Span> = Vec::new();
        for (i, block) in pcm.chunks(160).enumerate() {
            if block.iter().all(|&s| s == 0) {
                continue;
            }
            let (t0, t1) = (i as u64 * 10, i as u64 * 10 + 10);
            match spans.last_mut() {
                Some(last) if last.t1_ms + 100 >= t0 => last.t1_ms = t1,
                _ => spans.push(span(t0, t1)),
            }
        }
        spans.retain(|s| s.t1_ms - s.t0_ms >= 250);
        spans
    }

    struct FakeSpawner(Arc<Mutex<Seen>>);

    struct FakeChild(Option<oneshot::Sender<()>>, oneshot::Receiver<()>);

    impl WorkerChild for FakeChild {
        fn pid(&self) -> Option<u32> {
            Some(4242)
        }
        fn start_kill(&mut self) -> io::Result<()> {
            if let Some(kill) = self.0.take() {
                let _ = kill.send(());
            }
            Ok(())
        }
        fn wait(&mut self) -> Pin<Box<dyn Future<Output = io::Result<Option<i32>>> + Send + '_>> {
            Box::pin(async move {
                let _ = (&mut self.1).await;
                Ok(Some(0))
            })
        }
    }

    impl WorkerSpawner for FakeSpawner {
        fn spawn(&self) -> io::Result<WorkerProcess> {
            let (parent_in, worker_in) = tokio::io::duplex(1 << 20);
            let (worker_out, parent_out) = tokio::io::duplex(1 << 20);
            let (_worker_err, parent_err) = tokio::io::duplex(1 << 10);
            let (kill, killed) = oneshot::channel();
            let (exited, exit) = oneshot::channel();
            self.0.lock().unwrap().spawns += 1;
            let seen = Arc::clone(&self.0);
            tokio::spawn(async move {
                tokio::select! {
                    () = fake_worker(seen, worker_in, worker_out) => {}
                    _ = killed => {}
                }
                let _ = exited.send(());
            });
            Ok(WorkerProcess {
                stdin: Box::pin(parent_in),
                stdout: Box::pin(parent_out),
                stderr: Box::pin(parent_err),
                child: Box::new(FakeChild(Some(kill), exit)),
            })
        }

        fn program(&self) -> String {
            "/opt/fake/biorouter".into()
        }
    }

    async fn send(out: &mut DuplexStream, message: &FromWorker) {
        let _ = write_frame(out, &protocol::to_json(message), &[]).await;
    }

    fn samples(payload: &[u8]) -> Vec<i16> {
        payload
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect()
    }

    async fn fake_worker(seen: Arc<Mutex<Seen>>, mut stdin: DuplexStream, mut out: DuplexStream) {
        let hello = Hello {
            protocol: PROTOCOL_VERSION,
            whisper: "4da27c41".into(),
            ggml: "0.28.0".into(),
            backends: vec!["cpu".into()],
            cpu: cpu::detect().into(),
            isa: "test".into(),
            system_info: String::new(),
        };
        send(&mut out, &FromWorker::Hello(hello)).await;
        while let Ok(frame) = read_frame(&mut stdin).await {
            let message: ToWorker = serde_json::from_slice(&frame.json).unwrap();
            match message {
                ToWorker::Load(_) => {
                    let loaded = Loaded {
                        load_ms: 1,
                        warmup_ms: 0,
                        backend: "cpu".into(),
                        device: None,
                        rss_bytes: None,
                        n_vocab: 51_864,
                        multilingual: false,
                    };
                    send(&mut out, &FromWorker::Loaded(loaded)).await;
                }
                ToWorker::VadSegments(v) => {
                    seen.lock().unwrap().vads += 1;
                    let spans = loud_spans(&samples(&frame.payload));
                    send(
                        &mut out,
                        &FromWorker::Segments(Segments { id: v.id, spans }),
                    )
                    .await;
                }
                ToWorker::Transcribe(t) => {
                    let pcm = samples(&frame.payload);
                    let prompted = !t.prompt_terms.is_empty();
                    let (failing, decode_time, scripted, fit) = {
                        let mut seen = seen.lock().unwrap();
                        seen.decodes.push((pcm.len(), t.decode.no_context));
                        seen.prompts.push(t.prompt_terms.len());
                        let scripted = if prompted {
                            seen.prompted_text.clone()
                        } else {
                            seen.unprompted_text.clone()
                        };
                        (
                            seen.fail_decode == Some(seen.decodes.len()),
                            seen.decode_time,
                            scripted,
                            seen.fit,
                        )
                    };
                    tokio::time::sleep(decode_time).await;
                    if failing {
                        let error = ErrorMsg {
                            id: Some(t.id),
                            code: ErrorCode::Internal,
                            message: "decoder failed".into(),
                        };
                        send(&mut out, &FromWorker::Error(error)).await;
                        continue;
                    }
                    let len_ms = pcm.len() as u64 / 16;
                    let prompt_terms_used =
                        fit.unwrap_or(t.prompt_terms.len())
                            .min(t.prompt_terms.len()) as u32;
                    let (text, segments) = match scripted {
                        Some(text) => (
                            text.clone(),
                            vec![SegmentText {
                                t0_ms: 0,
                                t1_ms: len_ms,
                                text,
                            }],
                        ),
                        None => {
                            // Two words per second of loud audio, and a tag piece to drop.
                            let loud_ms: u64 =
                                loud_spans(&pcm).iter().map(|s| s.t1_ms - s.t0_ms).sum();
                            let words = vec!["word"; (loud_ms / 500) as usize].join(" ");
                            (
                                format!("[BLANK_AUDIO] {words}"),
                                vec![
                                    SegmentText {
                                        t0_ms: 0,
                                        t1_ms: len_ms / 2,
                                        text: " [BLANK_AUDIO]".into(),
                                    },
                                    SegmentText {
                                        t0_ms: len_ms / 2,
                                        t1_ms: len_ms,
                                        text: format!(" {words}"),
                                    },
                                ],
                            )
                        }
                    };
                    let result = TranscribeResult {
                        id: t.id,
                        text,
                        segments,
                        prompt_terms_used,
                        timings: Timings {
                            vad_ms: 1.0,
                            encode_ms: 10.0,
                            decode_ms: 5.0,
                        },
                    };
                    send(&mut out, &FromWorker::Result(result)).await;
                }
                ToWorker::Shutdown => return,
                _ => {}
            }
        }
    }

    fn supervisor() -> (Supervisor, Arc<Mutex<Seen>>) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let supervisor = Supervisor::start(
            Arc::new(FakeSpawner(Arc::clone(&seen))),
            SupervisorConfig::default(),
        );
        (supervisor, seen)
    }

    fn spec() -> Arc<LoadSpec> {
        Arc::new(LoadSpec {
            model_path: "/m/ggml-small.en-q8_0.bin".into(),
            model_sha256: "aa".into(),
            model_size: 1,
            vad_path: "/m/ggml-silero-v6.2.0.bin".into(),
            use_gpu: false,
            n_threads: 2,
        })
    }

    /// `ms` of silence, then a tone for every `(start, end)` in milliseconds.
    fn clip(ms: u64, tones: &[(u64, u64)]) -> Arc<[i16]> {
        let mut pcm = vec![0i16; (ms * 16) as usize];
        for &(start, end) in tones {
            for (i, sample) in pcm
                .iter_mut()
                .enumerate()
                .take((end * 16) as usize)
                .skip((start * 16) as usize)
            {
                *sample = if i % 32 < 16 { 6_000 } else { -6_000 };
            }
        }
        pcm.into()
    }

    /// DD-50 on a whole recording: a generated 40 s clip yields at least two decodes, each
    /// 25 s or less, split at the pauses the detector reports, each sent with
    /// `no_context = true`; the pieces come back in order and a tag piece is dropped.
    #[tokio::test]
    async fn a_40_s_clip_is_decoded_in_segments_of_25_s_or_less() {
        let (supervisor, seen) = supervisor();
        let pcm = clip(40_000, &[(500, 12_000), (13_500, 26_000), (27_000, 39_500)]);
        let transcript = transcribe_recording(
            &supervisor,
            spec(),
            pcm,
            TranscribeParams::new("en", 2),
            &CancellationToken::new(),
        )
        .await
        .expect("a transcript");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.vads, 1);
        assert!(seen.decodes.len() >= 2, "{:?}", seen.decodes);
        assert!(seen
            .decodes
            .iter()
            .all(|(n, no_context)| *n <= 25 * 16_000 && *no_context));
        assert!(!transcript.no_speech);
        assert_eq!(transcript.timings.decodes as usize, seen.decodes.len());
        assert!(transcript.text.starts_with("word word"));
        assert!(!transcript.text.contains("BLANK_AUDIO"));
        assert_eq!(transcript.segments.len(), seen.decodes.len());
        assert!(transcript
            .segments
            .windows(2)
            .all(|pair| pair[0].t1_ms <= pair[1].t0_ms));
        assert_eq!(transcript.audio_ms, 40_000);
        assert!(transcript.speech_ms > 33_000 && transcript.speech_ms < 38_000);
        assert_eq!(transcript.retried, None);
    }

    /// No request at all for exact zeros; one detector request and no decode for audio
    /// the detector hears nothing in.
    #[tokio::test]
    async fn nothing_heard_costs_no_decode() {
        let (supervisor, seen) = supervisor();
        let zeros = clip(5_000, &[]);
        let transcript = transcribe_recording(
            &supervisor,
            spec(),
            zeros,
            TranscribeParams::new("en", 2),
            &CancellationToken::new(),
        )
        .await
        .expect("a transcript");
        assert!(transcript.no_speech && transcript.text.is_empty());
        assert_eq!(
            seen.lock().unwrap().spawns,
            0,
            "exact zeros start no worker"
        );

        // A click shorter than the detector's 250 ms is not speech.
        let click = clip(5_000, &[(2_000, 2_100)]);
        let transcript = transcribe_recording(
            &supervisor,
            spec(),
            click,
            TranscribeParams::new("en", 2),
            &CancellationToken::new(),
        )
        .await
        .expect("a transcript");
        assert!(transcript.no_speech);
        let seen = seen.lock().unwrap();
        assert_eq!((seen.vads, seen.decodes.len()), (1, 0));
    }

    /// A recording longer than one detector window is detected window by window and every
    /// decode stays within 25 s.
    #[tokio::test]
    async fn a_long_recording_takes_several_windows() {
        let (supervisor, seen) = supervisor();
        let tones: Vec<(u64, u64)> = (0..70)
            .map(|i| (i * 10_000 + 1_000, i * 10_000 + 9_000))
            .collect();
        let pcm = clip(700_000, &tones);
        let transcript = transcribe_recording(
            &supervisor,
            spec(),
            pcm,
            TranscribeParams::new("en", 2),
            &CancellationToken::new(),
        )
        .await
        .expect("a transcript");
        let seen = seen.lock().unwrap();
        assert!(seen.vads >= 2, "{}", seen.vads);
        assert!(seen.decodes.iter().all(|(n, _)| *n <= 25 * 16_000));
        assert!(!transcript.no_speech);
        // 70 bursts of 8 s at two words per second, each decoded once.
        assert_eq!(text::word_count(&transcript.text), 70 * 16);
    }

    /// A decode that fails fails the recording, and the decodes queued behind it are
    /// dropped rather than run for a transcript nobody will get.
    #[tokio::test]
    async fn a_failed_decode_fails_the_recording_and_drops_the_rest() {
        let (supervisor, seen) = supervisor();
        {
            let mut seen = seen.lock().unwrap();
            seen.fail_decode = Some(2);
            seen.decode_time = Duration::from_millis(20);
        }
        let tones: Vec<(u64, u64)> = (0..60)
            .map(|i| (i * 10_000 + 1_000, i * 10_000 + 9_000))
            .collect();
        let result = transcribe_recording(
            &supervisor,
            spec(),
            clip(590_000, &tones),
            TranscribeParams::new("en", 2),
            &CancellationToken::new(),
        )
        .await;
        assert!(
            matches!(result, Err(DictationError::Internal(ref m)) if m.contains("decoder failed")),
            "{result:?}"
        );
        // What ran of the failed recording is a few decodes, not the 30 that were queued:
        // left in the queue, at 20 ms each, they would all have run within this.
        tokio::time::sleep(Duration::from_millis(1_500)).await;
        let ran = seen.lock().unwrap().decodes.len();
        // And the supervisor still serves.
        tokio::time::timeout(
            Duration::from_secs(5),
            transcribe_recording(
                &supervisor,
                spec(),
                clip(10_000, &[(1_000, 9_000)]),
                TranscribeParams::new("en", 2),
                &CancellationToken::new(),
            ),
        )
        .await
        .expect("in time")
        .expect("a transcript");
        assert!(ran <= 4, "{ran} decodes ran");
    }

    /// A piece Whisper timed past its segment still makes a valid cue: its start is never
    /// after its end, and both stay inside the segment.
    #[test]
    fn a_piece_timed_past_its_segment_stays_a_valid_cue() {
        let piece = |t0_ms, t1_ms| SegmentText {
            t0_ms,
            t1_ms,
            text: " Thank you.".into(),
        };
        let range = 30_000..32_000;
        assert_eq!(place(&range, &piece(300, 1_500)), (30_300, 31_500));
        assert_eq!(place(&range, &piece(1_500, 2_600)), (31_500, 32_000));
        // Both times past the segment: a zero length cue at its end, not 32,500 --> 32,000.
        assert_eq!(place(&range, &piece(2_500, 2_900)), (32_000, 32_000));
        assert_eq!(place(&range, &piece(900, 400)), (30_400, 30_400));
    }

    /// The prompted recording the retry tests share: `terms` sent with every decode of a
    /// 10 s clip that holds one stretch of speech.
    async fn prompted(
        supervisor: &Supervisor,
        terms: &[&str],
    ) -> Result<RecordingTranscript, DictationError> {
        let mut params = TranscribeParams::new("en", 2);
        params.prompt_terms = terms.iter().map(|t| t.to_string()).collect();
        transcribe_recording(
            supervisor,
            spec(),
            clip(10_000, &[(1_000, 9_000)]),
            params,
            &CancellationToken::new(),
        )
        .await
    }

    /// A decode that only echoes the prompt is decoded once more without it, and the
    /// retry's text is kept. Both checks read the prompt the worker fitted, not the whole
    /// list: the retry ends by naming the two terms that did not fit, which a check against
    /// the whole list would take for a recital of its head and throw away.
    #[tokio::test]
    async fn an_echo_is_retried_without_the_prompt_and_the_retry_is_kept() {
        let (supervisor, seen) = supervisor();
        {
            let mut seen = seen.lock().unwrap();
            seen.fit = Some(3);
            seen.prompted_text = Some(" Metformin, lisinopril, atorvastatin".into());
            seen.unprompted_text = Some(" Start the dose today. Alpha, bravo.".into());
        }
        let transcript = prompted(
            &supervisor,
            &["alpha", "bravo", "metformin", "lisinopril", "atorvastatin"],
        )
        .await
        .expect("a transcript");
        assert_eq!(seen.lock().unwrap().prompts, [5, 0]);
        assert_eq!(transcript.retried, Some(RetryReason::Echo));
        assert_eq!(transcript.timings.decodes, 2);
        assert!(!transcript.no_speech);
        assert_eq!(transcript.text, "Start the dose today. Alpha, bravo.");
    }

    /// A retry that echoes the prompt too is not adopted, so the recording is no speech.
    #[tokio::test]
    async fn a_retry_that_echoes_too_is_no_speech() {
        let (supervisor, seen) = supervisor();
        {
            let mut seen = seen.lock().unwrap();
            let echo = " Metformin, lisinopril, atorvastatin".to_string();
            seen.prompted_text = Some(echo.clone());
            seen.unprompted_text = Some(echo);
        }
        let transcript = prompted(&supervisor, &["metformin", "lisinopril", "atorvastatin"])
            .await
            .expect("a transcript");
        assert_eq!(seen.lock().unwrap().prompts, [3, 0]);
        assert_eq!(transcript.retried, Some(RetryReason::Echo));
        assert_eq!(transcript.timings.decodes, 2);
        assert!(transcript.no_speech && transcript.text.is_empty());
    }

    #[tokio::test]
    async fn cancelling_answers_cancelled_at_once() {
        let (supervisor, _) = supervisor();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = transcribe_recording(
            &supervisor,
            spec(),
            clip(10_000, &[(1_000, 9_000)]),
            TranscribeParams::new("en", 2),
            &cancel,
        )
        .await;
        assert!(
            matches!(result, Err(DictationError::Cancelled)),
            "{result:?}"
        );
        // The supervisor still serves the next recording.
        tokio::time::timeout(
            Duration::from_secs(5),
            transcribe_recording(
                &supervisor,
                spec(),
                clip(10_000, &[(1_000, 9_000)]),
                TranscribeParams::new("en", 2),
                &CancellationToken::new(),
            ),
        )
        .await
        .expect("in time")
        .expect("a transcript");
    }
}
