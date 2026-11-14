//! PCM helpers, the resampler and the segment planner (DD-28, DD-47, DD-48, DD-50).
//!
//! Audio moves through Biorouter as 16 kHz mono signed 16 bit samples: half the bytes of
//! `f32` on the worker's pipe, converted once, in the worker, into a reused `f32` buffer.
//! A [`PcmSlice`] shares one recording between every job cut from it, so a segment goes
//! from the caller's buffer to the pipe without a copy (lever L-8).
//!
//! The planner turns voice detector spans into decode segments while the person is
//! still speaking (lever L-7): every closed stretch of speech is queued as soon as it
//! ends, at most 25 s at a time, so at stop only the open tail is left to decode.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// The sample rate of every buffer the engine takes.
pub const SAMPLE_RATE: u32 = 16_000;
/// Samples per millisecond at [`SAMPLE_RATE`].
pub const SAMPLES_PER_MS: u64 = 16;
/// At most 25 s of audio per decode (DD-47).
pub const MAX_SEGMENT_SAMPLES: usize = 25 * 16_000;
/// [`MAX_SEGMENT_SAMPLES`] in milliseconds.
pub const MAX_SEGMENT_MS: u64 = 25_000;
/// At most 600 s of audio in one dictation or one detector request (DD-28).
pub const MAX_STREAM_SAMPLES: usize = 600 * 16_000;
/// While streaming, the parent asks for voice segments once this much audio lies past
/// the decoded mark (DD-50).
pub const STREAM_VAD_TRIGGER_MS: u64 = 8_000;
/// Audio kept before the first and after the last stretch of speech of a segment (DD-48).
pub const EDGE_PAD_MS: u64 = 300;
/// A span that ends this close to the end of the buffered audio may still be growing,
/// so it is left for a later call [I, M-19 tunes it].
pub const OPEN_TAIL_MS: u64 = 300;

/// A stretch of 16 kHz samples inside a shared recording. Cloning it clones a pointer.
#[derive(Clone)]
pub struct PcmSlice {
    pcm: Arc<[i16]>,
    start: usize,
    end: usize,
}

impl PcmSlice {
    /// The whole recording.
    pub fn whole(pcm: Arc<[i16]>) -> PcmSlice {
        let end = pcm.len();
        PcmSlice { pcm, start: 0, end }
    }

    /// `range` of `pcm`, in samples, clamped to the recording.
    pub fn new(pcm: Arc<[i16]>, range: Range<usize>) -> PcmSlice {
        let end = range.end.min(pcm.len());
        let start = range.start.min(end);
        PcmSlice { pcm, start, end }
    }

    /// The milliseconds `range` of the same recording, clamped to it.
    pub fn slice_ms(&self, range: &Range<u64>) -> PcmSlice {
        let to_sample =
            |ms: u64| usize::try_from(ms.saturating_mul(SAMPLES_PER_MS)).unwrap_or(usize::MAX);
        PcmSlice::new(
            Arc::clone(&self.pcm),
            to_sample(range.start)..to_sample(range.end),
        )
    }

    pub fn samples(&self) -> &[i16] {
        self.pcm.get(self.start..self.end).unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// The slice's length in milliseconds, rounded up.
    pub fn duration_ms(&self) -> u64 {
        (self.len() as u64).div_ceil(SAMPLES_PER_MS)
    }

    /// The slice's length in seconds, for the supervisor's deadlines.
    pub fn duration_secs(&self) -> f64 {
        self.len() as f64 / f64::from(SAMPLE_RATE)
    }
}

impl std::fmt::Debug for PcmSlice {
    /// Never the samples: audio does not belong in a log line or a test failure.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PcmSlice")
            .field("start", &self.start)
            .field("end", &self.end)
            .finish()
    }
}

/// A stretch of speech, in milliseconds from the start of the buffer it was found in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub t0_ms: u64,
    pub t1_ms: u64,
}

/// Why a PCM payload was refused. `Display` is a fixed phrase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioError {
    /// An odd number of bytes cannot be 16 bit samples.
    OddLength,
    TooLong {
        samples: usize,
        max: usize,
    },
    /// Not a RIFF file of 16 bit PCM.
    NotWav,
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::OddLength => f.write_str("audio is not 16 bit samples"),
            AudioError::TooLong { .. } => f.write_str("too much audio in one request"),
            AudioError::NotWav => f.write_str("the file is not 16 bit PCM WAV"),
        }
    }
}

/// A 16 bit PCM WAV file, its channels interleaved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WavS16 {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

impl WavS16 {
    /// The first channel only, at the file's own rate.
    pub fn first_channel(&self) -> Vec<i16> {
        self.samples
            .iter()
            .step_by(usize::from(self.channels.max(1)))
            .copied()
            .collect()
    }

    /// Mono 16 kHz, resampled when the file has another rate.
    pub fn to_16k_mono(&self) -> Vec<i16> {
        let first = self.first_channel();
        if self.sample_rate == SAMPLE_RATE {
            return first;
        }
        let mut floats = Vec::new();
        s16_to_f32(&first, &mut floats);
        f32_to_s16(&resample_to_16k(&floats, self.sample_rate))
    }
}

/// Reads a RIFF WAVE file of 16 bit PCM, skipping chunks other than `fmt ` and `data`.
/// The tools and tests use it; `biorouter transcribe` decodes files with its own reader.
pub fn parse_wav_s16(bytes: &[u8]) -> Result<WavS16, AudioError> {
    if bytes.get(..4) != Some(b"RIFF".as_slice()) || bytes.get(8..12) != Some(b"WAVE".as_slice()) {
        return Err(AudioError::NotWav);
    }
    let u16_at = |at: usize| -> Option<u16> {
        Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
    };
    let u32_at = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    let mut format = None;
    let mut at = 12usize;
    while let (Some(id), Some(len)) = (bytes.get(at..at + 4), u32_at(at + 4)) {
        let body = at + 8;
        let len = len as usize;
        match id {
            b"fmt " => {
                let pcm = u16_at(body) == Some(1) && u16_at(body + 14) == Some(16);
                let channels = u16_at(body + 2).filter(|c| *c > 0);
                let rate = u32_at(body + 4).filter(|r| *r > 0);
                format = pcm.then_some(channels.zip(rate)).flatten();
            }
            b"data" => {
                let (channels, sample_rate) = format.ok_or(AudioError::NotWav)?;
                // A writer that never patched the length leaves it at 0 or past the end.
                let data = bytes
                    .get(body..body.saturating_add(len).min(bytes.len()))
                    .ok_or(AudioError::NotWav)?;
                return Ok(WavS16 {
                    sample_rate,
                    channels,
                    samples: data
                        .chunks_exact(2)
                        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                        .collect(),
                });
            }
            _ => {}
        }
        at = body.saturating_add(len).saturating_add(len & 1);
    }
    Err(AudioError::NotWav)
}

impl std::error::Error for AudioError {}

/// The samples as little endian bytes. Borrowed, not copied, on every little endian
/// target, which is every target Biorouter ships.
pub fn s16le_bytes(pcm: &[i16]) -> Cow<'_, [u8]> {
    #[cfg(target_endian = "little")]
    {
        // SAFETY: `i16` has no padding and every byte pattern is a valid `u8`; the byte
        // view covers exactly the slice's memory and borrows it for the same lifetime.
        Cow::Borrowed(unsafe {
            std::slice::from_raw_parts(pcm.as_ptr().cast::<u8>(), std::mem::size_of_val(pcm))
        })
    }
    #[cfg(not(target_endian = "little"))]
    {
        Cow::Owned(pcm.iter().flat_map(|s| s.to_le_bytes()).collect())
    }
}

/// `x / 32768` for every sample, into a buffer whose capacity is reused.
pub fn s16_to_f32(src: &[i16], dst: &mut Vec<f32>) {
    dst.clear();
    dst.extend(src.iter().map(|&s| f32::from(s) / 32_768.0));
}

/// [`s16_to_f32`] straight from a little endian payload, with no `i16` copy between.
pub fn s16le_to_f32(bytes: &[u8], dst: &mut Vec<f32>) {
    dst.clear();
    dst.extend(
        bytes
            .chunks_exact(2)
            .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32_768.0),
    );
}

/// The sample count of a little endian 16 bit payload, refused when it is odd or longer
/// than `max_samples`.
pub fn validate_s16le(bytes: &[u8], max_samples: usize) -> Result<usize, AudioError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(AudioError::OddLength);
    }
    let samples = bytes.len() / 2;
    if samples > max_samples {
        return Err(AudioError::TooLong {
            samples,
            max: max_samples,
        });
    }
    Ok(samples)
}

/// Clamped and rounded to 16 bit samples.
pub fn f32_to_s16(src: &[f32]) -> Vec<i16> {
    src.iter()
        .map(|&x| {
            let scaled = (f64::from(x) * 32_768.0).round();
            scaled.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
        })
        .collect()
}

/// Zero crossings of the sinc kernel on each side of the centre.
const ZERO_CROSSINGS: f64 = 32.0;
/// The cutoff as a fraction of the lower of the two rates: 7.2 kHz for 16 kHz output.
const CUTOFF_FRACTION: f64 = 0.45;
/// Above this many phases the kernel is computed per sample rather than tabulated.
const MAX_TABLE_PHASES: u64 = 2_048;

/// Resamples to 16 kHz with a Blackman windowed sinc low pass at 7.2 kHz (or 0.45 of a
/// lower input rate) and 32 zero crossings on each side. Rates whose ratio to 16 kHz
/// reduces to at most 2,048 phases (every common rate) use a precomputed polyphase
/// table, so each output sample costs one dot product.
pub fn resample_to_16k(src: &[f32], from_rate: u32) -> Vec<f32> {
    resample(src, from_rate, SAMPLE_RATE)
}

fn resample(src: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == 0 || to == 0 || src.is_empty() {
        return Vec::new();
    }
    if from == to {
        return src.to_vec();
    }
    let (from_f, to_f) = (f64::from(from), f64::from(to));
    let out_len = ((src.len() as f64) * to_f / from_f).round() as usize;
    let cutoff = CUTOFF_FRACTION * from_f.min(to_f);
    // The kernel in input samples: sinc(2 fc d / from) has a zero every from / (2 fc).
    let step = 2.0 * cutoff / from_f;
    let half_width = ZERO_CROSSINGS / step;
    let w = half_width.ceil() as i64;
    let taps = (2 * w) as usize;
    let g = gcd(u64::from(from), u64::from(to));
    let (up, down) = (u64::from(to) / g, u64::from(from) / g);

    let kernel = |d: f64| -> f64 {
        let u = d / half_width;
        if u.abs() >= 1.0 {
            return 0.0;
        }
        let window = 0.42
            + 0.5 * (std::f64::consts::PI * u).cos()
            + 0.08 * (2.0 * std::f64::consts::PI * u).cos();
        let x = step * d;
        let sinc = if x.abs() < 1e-12 {
            1.0
        } else {
            (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
        };
        step * sinc * window
    };
    // The taps for one fractional position, normalised to a gain of exactly 1 at DC.
    let phase_taps = |frac: f64, out: &mut Vec<f32>| {
        out.clear();
        let mut sum = 0.0;
        let start = out.len();
        for j in (1 - w)..=w {
            let value = kernel(frac - j as f64);
            sum += value;
            out.push(value as f32);
        }
        if sum.abs() > 1e-12 {
            for value in out.iter_mut().skip(start) {
                *value = (f64::from(*value) / sum) as f32;
            }
        }
    };

    let table: Option<Vec<f32>> = (up <= MAX_TABLE_PHASES).then(|| {
        let mut table = Vec::with_capacity(taps * up as usize);
        let mut row = Vec::with_capacity(taps);
        for p in 0..up {
            phase_taps(p as f64 / up as f64, &mut row);
            table.extend_from_slice(&row);
        }
        table
    });

    let len = src.len() as i64;
    let mut out = Vec::with_capacity(out_len);
    let mut row = Vec::with_capacity(taps);
    for n in 0..out_len as u64 {
        let position = n * down;
        let i = (position / up) as i64;
        let phase = position % up;
        let coeffs: &[f32] = match &table {
            Some(table) => {
                let at = phase as usize * taps;
                table.get(at..at + taps).unwrap_or_default()
            }
            None => {
                phase_taps(phase as f64 / up as f64, &mut row);
                &row
            }
        };
        let k0 = i - w + 1;
        let acc = if k0 >= 0 && k0 + taps as i64 <= len {
            src.get(k0 as usize..k0 as usize + taps)
                .unwrap_or_default()
                .iter()
                .zip(coeffs)
                .map(|(s, c)| s * c)
                .sum::<f32>()
        } else {
            coeffs
                .iter()
                .enumerate()
                .filter_map(|(j, c)| {
                    let k = usize::try_from(k0 + j as i64).ok()?;
                    src.get(k).map(|s| s * c)
                })
                .sum::<f32>()
        };
        out.push(acc);
    }
    out
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// DD-50, while the person is speaking. `spans` are the detector's spans in the stream
/// buffer's own milliseconds; everything before `decoded_mark_ms` is already queued.
/// Returns the closed segments to queue now and the new decoded mark.
///
/// Nothing is planned until [`STREAM_VAD_TRIGGER_MS`] lie past the mark. A span that
/// ends within [`OPEN_TAIL_MS`] of `buffered_end_ms` is still being spoken and waits.
/// Every closed span found in one call forms one segment, padded by [`EDGE_PAD_MS`]
/// without reaching into the next span, and split only to respect the 25 s cap: at the
/// longest pause, or, for speech with no pause, every 25 s.
pub fn plan_closed_segments(
    spans: &[Span],
    decoded_mark_ms: u64,
    buffered_end_ms: u64,
) -> (Vec<Range<u64>>, u64) {
    if buffered_end_ms < decoded_mark_ms.saturating_add(STREAM_VAD_TRIGGER_MS) {
        return (Vec::new(), decoded_mark_ms);
    }
    let live = clip(spans, decoded_mark_ms, buffered_end_ms);
    let closed = live
        .iter()
        .take_while(|s| s.t1_ms.saturating_add(OPEN_TAIL_MS) <= buffered_end_ms)
        .count();
    let (Some(first), Some(last)) = (live.first(), live.get(closed.wrapping_sub(1))) else {
        return (Vec::new(), decoded_mark_ms);
    };
    if closed == 0 {
        return (Vec::new(), decoded_mark_ms);
    }
    let start = padded_start(first.t0_ms, decoded_mark_ms);
    let next = live.get(closed).map(|s| s.t0_ms);
    let end = padded_end(last.t1_ms, next, buffered_end_ms);
    let segments = split(live.get(..closed).unwrap_or_default(), start, end);
    (segments, end)
}

/// DD-50, at stop: every span left after the mark is closed, so all of it is planned.
pub fn plan_final_segments(
    spans: &[Span],
    decoded_mark_ms: u64,
    buffered_end_ms: u64,
) -> Vec<Range<u64>> {
    let live = clip(spans, decoded_mark_ms, buffered_end_ms);
    let (Some(first), Some(last)) = (live.first(), live.last()) else {
        return Vec::new();
    };
    let start = padded_start(first.t0_ms, decoded_mark_ms);
    let end = padded_end(last.t1_ms, None, buffered_end_ms);
    split(&live, start, end)
}

/// The spans that lie after `mark`, cut to `[mark, end)` and sorted.
fn clip(spans: &[Span], mark: u64, end: u64) -> Vec<Span> {
    let mut live: Vec<Span> = spans
        .iter()
        .filter(|s| s.t1_ms > mark && s.t0_ms < end)
        .map(|s| Span {
            t0_ms: s.t0_ms.max(mark),
            t1_ms: s.t1_ms.min(end),
        })
        .filter(|s| s.t0_ms < s.t1_ms)
        .collect();
    live.sort_by_key(|s| s.t0_ms);
    live
}

fn padded_start(t0: u64, floor: u64) -> u64 {
    t0.saturating_sub(EDGE_PAD_MS).max(floor)
}

/// `t1` plus the pad, never past `ceiling` and never past the middle of the pause before
/// the next span, so the next segment's own pad reaches no further back than that.
fn padded_end(t1: u64, next_t0: Option<u64>, ceiling: u64) -> u64 {
    let mut end = t1.saturating_add(EDGE_PAD_MS).min(ceiling);
    if let Some(next) = next_t0.filter(|next| *next >= t1) {
        end = end.min(t1 + (next - t1) / 2);
    }
    end.max(t1.min(ceiling))
}

/// `[start, end)` as segments of at most 25 s: split at the longest pause between spans,
/// or cut every 25 s inside a single span.
fn split(spans: &[Span], start: u64, end: u64) -> Vec<Range<u64>> {
    if end <= start {
        return Vec::new();
    }
    if end - start <= MAX_SEGMENT_MS {
        return std::iter::once(start..end).collect();
    }
    let longest_pause = spans
        .windows(2)
        .enumerate()
        .max_by_key(|(i, pair)| {
            let gap = pair[1].t0_ms.saturating_sub(pair[0].t1_ms);
            // The first of equal pauses wins, which keeps the plan deterministic.
            (gap, std::cmp::Reverse(*i))
        })
        .map(|(i, _)| i + 1);
    match longest_pause {
        Some(at) => {
            let (left, right) = spans.split_at(at);
            let (a, b) = (
                left.last().map_or(start, |s| s.t1_ms),
                right.first().map_or(end, |s| s.t0_ms),
            );
            let left_end = padded_end(a, Some(b), end);
            let right_start = b
                .saturating_sub(EDGE_PAD_MS)
                .max(left_end)
                .max(start)
                .min(b);
            let mut segments = split(left, start, left_end);
            segments.extend(split(right, right_start, end));
            segments
        }
        None => (start..end)
            .step_by(MAX_SEGMENT_MS as usize)
            .map(|t| t..(t + MAX_SEGMENT_MS).min(end))
            .collect(),
    }
}

#[cfg(test)]
// A plan of one segment is a list holding one range.
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    fn span(t0_ms: u64, t1_ms: u64) -> Span {
        Span { t0_ms, t1_ms }
    }

    #[test]
    fn s16_and_f32_round_trip() {
        let samples = [i16::MIN, -16_384, -1, 0, 1, 16_384, i16::MAX];
        let mut floats = Vec::new();
        s16_to_f32(&samples, &mut floats);
        assert_eq!(floats.first(), Some(&-1.0));
        assert_eq!(floats.get(3), Some(&0.0));
        assert_eq!(f32_to_s16(&floats), samples);

        let bytes = s16le_bytes(&samples);
        assert_eq!(bytes.len(), samples.len() * 2);
        assert_eq!(bytes.get(..2), Some(i16::MIN.to_le_bytes().as_slice()));
        let mut from_bytes = vec![9.0; 3];
        s16le_to_f32(&bytes, &mut from_bytes);
        assert_eq!(from_bytes, floats);

        assert_eq!(f32_to_s16(&[2.0, -2.0, 0.5]), [i16::MAX, i16::MIN, 16_384]);
    }

    #[test]
    fn payloads_are_validated() {
        assert_eq!(validate_s16le(&[0; 6], 3), Ok(3));
        assert_eq!(validate_s16le(&[0; 5], 3), Err(AudioError::OddLength));
        assert_eq!(
            validate_s16le(&[0; 8], 3),
            Err(AudioError::TooLong { samples: 4, max: 3 })
        );
        assert_eq!(validate_s16le(&[], 3), Ok(0));
    }

    #[test]
    fn wav_files_are_read_past_extra_chunks() {
        // jfk.wav carries a LIST chunk between `fmt ` and `data`.
        let bytes = std::fs::read(crate::build_info::fixtures::path(
            crate::build_info::fixtures::JFK,
        ))
        .expect("jfk.wav");
        let wav = parse_wav_s16(&bytes).expect("16 bit PCM");
        assert_eq!((wav.sample_rate, wav.channels), (16_000, 1));
        assert_eq!(wav.samples.len(), 176_000);
        assert_eq!(wav.to_16k_mono().len(), 176_000);

        let mut stereo = Vec::new();
        stereo.extend_from_slice(b"RIFF\0\0\0\0WAVEfmt ");
        stereo.extend_from_slice(&16u32.to_le_bytes());
        for v in [1u16, 2] {
            stereo.extend_from_slice(&v.to_le_bytes());
        }
        stereo.extend_from_slice(&48_000u32.to_le_bytes());
        stereo.extend_from_slice(&192_000u32.to_le_bytes());
        for v in [4u16, 16] {
            stereo.extend_from_slice(&v.to_le_bytes());
        }
        stereo.extend_from_slice(b"data");
        stereo.extend_from_slice(&0u32.to_le_bytes());
        for i in 0..4_800i16 {
            stereo.extend_from_slice(&i.to_le_bytes());
            stereo.extend_from_slice(&(-i).to_le_bytes());
        }
        // A data length of 0 (a writer that never patched it) reads nothing.
        let wav = parse_wav_s16(&stereo).expect("a stereo file");
        assert_eq!((wav.sample_rate, wav.channels), (48_000, 2));
        assert!(wav.samples.is_empty());

        let len_at = stereo.len() - 4 * 4_800 - 4;
        stereo
            .get_mut(len_at..len_at + 4)
            .expect("the data length")
            .copy_from_slice(&(4u32 * 4_800).to_le_bytes());
        let wav = parse_wav_s16(&stereo).expect("a stereo file");
        assert_eq!(wav.first_channel().len(), 4_800);
        assert_eq!(wav.first_channel().get(7), Some(&7));
        assert_eq!(wav.to_16k_mono().len(), 1_600);
        assert_eq!(parse_wav_s16(b"RIFX0000WAVE"), Err(AudioError::NotWav));
    }

    #[test]
    fn a_slice_shares_its_recording() {
        let pcm: Arc<[i16]> = (0..32_000).map(|i| i as i16).collect();
        let whole = PcmSlice::whole(Arc::clone(&pcm));
        assert_eq!(whole.len(), 32_000);
        assert_eq!(whole.duration_ms(), 2_000);
        let part = whole.slice_ms(&(500..1_000));
        assert_eq!(part.len(), 8_000);
        assert_eq!(part.samples().first(), Some(&8_000));
        assert!(Arc::ptr_eq(&pcm, &part.pcm));
        let past_the_end = whole.slice_ms(&(1_900..9_000));
        assert_eq!(past_the_end.len(), 1_600);
        assert!(whole.slice_ms(&(3_000..4_000)).is_empty());
        assert_eq!(format!("{part:?}"), "PcmSlice { start: 8000, end: 16000 }");
    }

    fn sine(freq: f64, rate: u32, seconds: f64) -> Vec<f32> {
        let n = (f64::from(rate) * seconds) as usize;
        (0..n)
            .map(|i| {
                (0.5 * (2.0 * std::f64::consts::PI * freq * i as f64 / f64::from(rate)).sin())
                    as f32
            })
            .collect()
    }

    fn zero_crossings(x: &[f32]) -> usize {
        x.windows(2)
            .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
            .count()
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
    }

    #[test]
    fn the_resampler_keeps_a_1_khz_tone() {
        for rate in [48_000u32, 44_100, 22_050, 8_000] {
            let input = sine(1_000.0, rate, 2.0);
            let output = resample_to_16k(&input, rate);
            let expected_len = 32_000i64;
            assert!(
                (output.len() as i64 - expected_len).abs() <= 2,
                "{rate}: {} samples",
                output.len()
            );
            let middle = output.get(1_600..30_400).expect("the middle");
            let crossings = zero_crossings(middle) as f64;
            let expected = 2.0 * 1_000.0 * middle.len() as f64 / 16_000.0;
            assert!(
                (crossings - expected).abs() <= expected * 0.01,
                "{rate}: {crossings} zero crossings, expected {expected}"
            );
            let gain_db = 20.0 * (rms(middle) / rms(&input)).log10();
            assert!(gain_db.abs() < 0.5, "{rate}: passband gain {gain_db:.2} dB");
        }
    }

    #[test]
    fn the_resampler_removes_what_16_khz_cannot_hold() {
        for rate in [48_000u32, 44_100] {
            let input = sine(9_000.0, rate, 1.0);
            let output = resample_to_16k(&input, rate);
            let middle = output.get(1_600..14_400).expect("the middle");
            let attenuation_db = 20.0 * (rms(middle) / rms(&input)).log10();
            assert!(
                attenuation_db < -40.0,
                "{rate}: a 9 kHz tone only fell {attenuation_db:.1} dB"
            );
        }
        assert_eq!(resample_to_16k(&[0.25; 10], 16_000), vec![0.25; 10]);
        assert!(resample_to_16k(&[0.25; 10], 0).is_empty());
    }

    #[test]
    fn nothing_is_planned_before_8_s() {
        let spans = [span(500, 2_000), span(3_000, 4_000)];
        assert_eq!(plan_closed_segments(&spans, 0, 7_999), (Vec::new(), 0));
        assert_eq!(
            plan_closed_segments(&spans, 10_000, 17_000),
            (Vec::new(), 10_000)
        );
    }

    #[test]
    fn an_open_tail_stays() {
        // The last span ends 200 ms before the buffered end: still being spoken.
        let spans = [span(1_000, 3_000), span(4_000, 8_800)];
        let (segments, mark) = plan_closed_segments(&spans, 0, 9_000);
        assert_eq!(segments, vec![700..3_300]);
        assert_eq!(mark, 3_300);
        // Only speech that is still open: nothing yet.
        let (segments, mark) = plan_closed_segments(&[span(2_000, 8_900)], 0, 9_000);
        assert!(segments.is_empty());
        assert_eq!(mark, 0);
        // At stop the open span is closed and decoded, starting at the mark.
        assert_eq!(
            plan_final_segments(&spans, 3_300, 9_000),
            vec![3_700..9_000]
        );
    }

    #[test]
    fn closed_spans_form_one_padded_segment() {
        let spans = [span(1_000, 3_000), span(3_500, 6_000), span(7_000, 8_000)];
        let (segments, mark) = plan_closed_segments(&spans, 0, 12_000);
        assert_eq!(segments, vec![700..8_300]);
        assert_eq!(mark, 8_300);
        // The next call starts at the mark and never decodes the same audio twice.
        let more = [span(7_000, 8_000), span(9_000, 15_000)];
        let (segments, mark) = plan_closed_segments(&more, 8_300, 20_000);
        assert_eq!(segments, vec![8_700..15_300]);
        assert_eq!(mark, 15_300);
        assert!(plan_final_segments(&[], 0, 5_000).is_empty());
    }

    #[test]
    fn pads_never_reach_into_the_next_span() {
        // 400 ms between the closed span and the open one: the pad stops halfway.
        let spans = [span(1_000, 7_000), span(7_400, 11_900)];
        let (segments, mark) = plan_closed_segments(&spans, 0, 12_000);
        assert_eq!(segments, vec![700..7_200]);
        assert_eq!(mark, 7_200);
        let tail = plan_final_segments(&spans, mark, 12_000);
        assert_eq!(tail, vec![7_200..12_000]);
        assert!(tail.iter().all(|r| r.start >= 7_000));
        // The mark is a floor for the start pad too.
        assert_eq!(
            plan_final_segments(&[span(100, 900)], 0, 5_000),
            vec![0..1_200]
        );
    }

    #[test]
    fn a_long_stretch_splits_at_the_longest_pause() {
        let spans = [
            span(0, 9_000),
            span(9_300, 15_000),
            // The longest pause: 1.2 s.
            span(16_200, 24_000),
            span(24_500, 31_000),
        ];
        let segments = plan_final_segments(&spans, 0, 32_000);
        assert_eq!(segments, vec![0..15_300, 15_900..31_300]);
        assert!(segments.iter().all(|r| r.end - r.start <= MAX_SEGMENT_MS));
        // Neither half reaches into the other's speech.
        assert!(segments[0].end <= 16_200 && segments[1].start >= 15_000);
    }

    #[test]
    fn speech_with_no_pause_is_cut_every_25_s() {
        let segments = plan_final_segments(&[span(1_000, 41_000)], 0, 45_000);
        assert_eq!(segments, vec![700..25_700, 25_700..41_300]);
        let (closed, mark) = plan_closed_segments(&[span(1_000, 41_000)], 0, 50_000);
        assert_eq!(closed, segments);
        assert_eq!(mark, 41_300);
    }

    #[test]
    fn every_planned_segment_fits_the_decode_window() {
        // A pseudo random pattern of speech and pauses over 600 s.
        let mut spans = Vec::new();
        let mut t = 0u64;
        let mut seed = 0x2545_f491u64;
        while t < 600_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let speech = 250 + seed % 30_000;
            let pause = 100 + (seed >> 20) % 1_500;
            spans.push(span(t, (t + speech).min(600_000)));
            t += speech + pause;
        }
        let segments = plan_final_segments(&spans, 0, 600_000);
        assert!(!segments.is_empty());
        for pair in segments.windows(2) {
            assert!(pair[0].end <= pair[1].start, "{pair:?} overlap");
        }
        for segment in &segments {
            assert!(segment.end - segment.start <= MAX_SEGMENT_MS, "{segment:?}");
        }
        // Every span's speech is inside some segment.
        for s in &spans {
            for t in [s.t0_ms, s.t1_ms - 1] {
                assert!(
                    segments.iter().any(|r| r.contains(&t)),
                    "{t} of {s:?} is not planned"
                );
            }
        }
    }
}
