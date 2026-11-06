//! Reading a recording for `biorouter transcribe` (DD-78).
//!
//! symphonia decodes, in this crate only: WAV of any rate, channel count and depth
//! (integer, float, A-law and mu-law), AIFF, FLAC and MP3. The engine and the daemon take
//! 16 kHz mono PCM and never parse a file. M4A, AAC and Ogg Vorbis are left out by
//! DD-78's size rule (the crate manifest has the measurement); they, Opus and WebM are
//! refused before anything is decoded, with an `ffmpeg` line that converts them.
//!
//! Channels are averaged as each packet is decoded, and the mono samples are resampled to
//! 16 kHz in blocks of 30 s with enough context on both sides that the result is the
//! one the whole recording resampled at once would give, so a two hour recording never
//! holds more than about a block at its own rate in memory. A recording over 2 hours is
//! refused before it is decoded when its header says so, and as soon as the decoding
//! passes 2 hours when it does not; that bounds memory, whatever the file's size. A file
//! is streamed from disk, so its size is not limited: 2 hours of 48 kHz stereo 24 bit
//! WAV is about 2 GB. Standard input has no seek, so it is read whole before it is
//! decoded, and refused past 200 MB.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use biorouter::dictation::audio::{f32_to_s16, resample_to_16k, SAMPLE_RATE};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use super::Outcome;

/// The most `transcribe -` reads from standard input, which it holds whole: 200 MB.
pub const MAX_INPUT_BYTES: u64 = 200_000_000;
/// The longest recording: 2 hours.
pub const MAX_SECONDS: u64 = 2 * 60 * 60;
/// The shortest: half a second (MSG-TOO-SHORT).
pub const MIN_MS: u64 = 500;
/// The resampler's block, in seconds of the source.
const BLOCK_SECONDS: usize = 30;

/// Why a recording could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    NotFound,
    Unreadable,
    /// A kind Biorouter does not decode, named for the person when it can be (`Opus`,
    /// `WebM`, `M4A and AAC`, `Ogg`, or the file's extension); `None` for bytes no reader
    /// knows that came with no name.
    UnsupportedFormat {
        kind: Option<String>,
    },
    TooLong,
    TooShort,
}

impl DecodeError {
    pub fn outcome(&self) -> Outcome {
        match self {
            DecodeError::NotFound => Outcome::NotFound,
            DecodeError::Unreadable => Outcome::Unreadable,
            DecodeError::UnsupportedFormat { .. } => Outcome::UnsupportedFormat,
            DecodeError::TooLong => Outcome::TooLong,
            DecodeError::TooShort => Outcome::TooShort,
        }
    }

    /// The sentence to show, for the file the person named (`-` for standard input).
    pub fn message(&self, input: &str) -> String {
        use super::messages;
        match self {
            DecodeError::NotFound => messages::msg_not_found(input),
            DecodeError::Unreadable => messages::msg_unreadable(input),
            DecodeError::UnsupportedFormat { kind } => {
                messages::msg_unsupported_format(kind.as_deref(), input)
            }
            DecodeError::TooLong => messages::msg_too_long(input),
            DecodeError::TooShort => messages::MSG_TOO_SHORT.to_string(),
        }
    }
}

fn refused(kind: &str) -> DecodeError {
    DecodeError::UnsupportedFormat {
        kind: Some(kind.to_string()),
    }
}

/// A recording as the engine takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// 16 kHz mono.
    pub samples: Vec<i16>,
    pub source_rate: u32,
    pub channels: usize,
}

impl Decoded {
    pub fn duration_ms(&self) -> u64 {
        (self.samples.len() as u64).div_ceil(u64::from(SAMPLE_RATE) / 1_000)
    }

    /// MSG-TOO-SHORT under half a second.
    pub fn check_length(&self) -> Result<(), DecodeError> {
        if self.duration_ms() < MIN_MS {
            return Err(DecodeError::TooShort);
        }
        Ok(())
    }

    /// Every sample exactly zero: no speech, decided without the engine (DD-48).
    pub fn is_digital_silence(&self) -> bool {
        self.samples.iter().all(|&s| s == 0)
    }
}

/// The extensions of the kinds Biorouter reads. A file named so that is still refused holds
/// something else (an ADPCM WAV from a voice recorder, compressed AIFF-C, an M4A renamed),
/// so its extension does not name what was refused, and the message names no kind.
const READ_EXTENSIONS: [&str; 7] = ["wav", "wave", "aif", "aiff", "aifc", "flac", "mp3"];

/// The kinds Biorouter does not decode, by the name of the file.
fn refused_extension(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "opus" => Some("Opus"),
        "webm" | "weba" => Some("WebM"),
        "m4a" | "mp4" | "m4b" | "aac" => Some("M4A and AAC"),
        "ogg" | "oga" => Some("Ogg"),
        _ => None,
    }
}

/// The same kinds by their first bytes, for standard input or a misnamed file:
/// Matroska's EBML magic, an MP4 `ftyp` box, an Ogg page (an Opus header in it names
/// Opus).
fn refused_magic(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some("WebM");
    }
    if head.starts_with(b"OggS") {
        if head.windows(8).any(|w| w == b"OpusHead") {
            return Some("Opus");
        }
        return Some("Ogg");
    }
    if head.get(4..8) == Some(b"ftyp".as_slice()) {
        return Some("M4A and AAC");
    }
    None
}

/// Reads and decodes the file at `path`, streaming it from disk: its size is not limited,
/// only the length of the audio it holds.
pub fn decode_path(path: &Path) -> Result<Decoded, DecodeError> {
    let metadata = std::fs::metadata(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => DecodeError::NotFound,
        _ => DecodeError::Unreadable,
    })?;
    if !metadata.is_file() {
        return Err(DecodeError::Unreadable);
    }
    let mut file = std::fs::File::open(path).map_err(|_| DecodeError::Unreadable)?;
    let mut head = [0u8; 64];
    let got = read_up_to(&mut file, &mut head).map_err(|_| DecodeError::Unreadable)?;
    // The bytes name the kind more exactly than the name does (an `.ogg` holding Opus).
    if let Some(kind) =
        refused_magic(head.get(..got).unwrap_or_default()).or_else(|| refused_extension(path))
    {
        return Err(refused(kind));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| DecodeError::Unreadable)?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let kind = ext
        .as_deref()
        .filter(|e| !READ_EXTENSIONS.contains(e))
        .map(|e| format!(".{e}"));
    decode(Box::new(file), ext.as_deref(), kind)
}

/// Reads all of `reader` (standard input for `transcribe -`), refusing more than 200 MB,
/// and decodes it. With no file name the container is found from its bytes.
pub fn decode_reader(reader: impl Read) -> Result<Decoded, DecodeError> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| DecodeError::Unreadable)?;
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(DecodeError::TooLong);
    }
    if let Some(kind) = refused_magic(bytes.get(..64.min(bytes.len())).unwrap_or_default()) {
        return Err(refused(kind));
    }
    decode(Box::new(std::io::Cursor::new(bytes)), None, None)
}

fn read_up_to(reader: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match reader.read(buf.get_mut(got..).unwrap_or_default())? {
            0 => break,
            n => got += n,
        }
    }
    Ok(got)
}

fn decode(
    source: Box<dyn MediaSource>,
    ext: Option<&str>,
    kind: Option<String>,
) -> Result<Decoded, DecodeError> {
    let unsupported = || DecodeError::UnsupportedFormat { kind: kind.clone() };
    let stream = MediaSourceStream::new(source, Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = ext {
        hint.with_extension(ext);
    }
    // Gapless: the encoder's delay and padding (an MP3's LAME header, an M4A's edit
    // list) are trimmed, so the audio is as long as what was recorded.
    let options = FormatOptions {
        enable_gapless: true,
        ..FormatOptions::default()
    };
    let probed = symphonia::default::get_probe()
        .format(&hint, stream, &options, &MetadataOptions::default())
        .map_err(|_| unsupported())?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(unsupported)?;
    let track_id = track.id;
    let params = track.codec_params.clone();
    if let (Some(frames), Some(rate)) = (params.n_frames, params.sample_rate) {
        if rate > 0 && frames > MAX_SECONDS * u64::from(rate) {
            return Err(DecodeError::TooLong);
        }
    }
    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .map_err(|_| unsupported())?;

    let mut resampler: Option<To16k> = None;
    let mut channels = 0usize;
    let mut frames_in: u64 = 0;
    let mut buffer: Option<(SampleBuffer<f32>, usize)> = None;
    let mut mono: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break
            }
            Err(SymphoniaError::ResetRequired) => break,
            // A file cut short or damaged after some audio: keep what was decoded.
            Err(_) if frames_in > 0 => break,
            Err(_) => return Err(unsupported()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let audio = match decoder.decode(&packet) {
            Ok(audio) => audio,
            // One damaged packet is skipped, as players do.
            Err(SymphoniaError::DecodeError(_) | SymphoniaError::IoError(_)) => continue,
            Err(SymphoniaError::ResetRequired) => break,
            Err(_) => return Err(unsupported()),
        };
        let spec = *audio.spec();
        let count = spec.channels.count().max(1);
        let rate = spec.rate;
        if rate == 0 {
            return Err(unsupported());
        }
        let to16k = resampler.get_or_insert_with(|| To16k::new(rate));
        if to16k.from != rate || (channels != 0 && channels != count) {
            // A stream that changes its rate or channels midway.
            return Err(unsupported());
        }
        channels = count;
        let capacity = audio.capacity();
        let sample_buffer = match buffer.as_mut() {
            Some((sample_buffer, have)) if *have >= capacity => sample_buffer,
            _ => {
                buffer = Some((SampleBuffer::<f32>::new(capacity as u64, spec), capacity));
                &mut buffer.as_mut().ok_or_else(unsupported)?.0
            }
        };
        sample_buffer.copy_interleaved_ref(audio);
        let interleaved = sample_buffer.samples();
        mono.clear();
        mono.extend(
            interleaved
                .chunks_exact(count)
                .map(|frame| frame.iter().sum::<f32>() / count as f32),
        );
        frames_in += mono.len() as u64;
        if frames_in > MAX_SECONDS * u64::from(rate) {
            return Err(DecodeError::TooLong);
        }
        to16k.push(&mono);
    }
    let Some(resampler) = resampler else {
        return Err(unsupported());
    };
    let source_rate = resampler.from;
    Ok(Decoded {
        samples: resampler.finish(),
        source_rate,
        channels,
    })
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Mono samples at the source rate in, 16 kHz samples out, a block at a time.
///
/// Block boundaries fall on multiples of the rate ratio's denominator, so every output
/// sample of a block sits at the same fractional position, with the same kernel phase,
/// as in a resampling of the whole recording; each block carries `margin` input samples
/// of context on both sides, more than the kernel's reach, so the output is the same.
struct To16k {
    from: u32,
    up: usize,
    down: usize,
    margin: usize,
    block: usize,
    /// Input from the absolute sample index `start`.
    input: Vec<f32>,
    start: usize,
    /// The absolute index of the first input sample whose output is still owed.
    next: usize,
    out: Vec<i16>,
}

impl To16k {
    fn new(from: u32) -> To16k {
        To16k::with_block(from, BLOCK_SECONDS * from as usize)
    }

    fn with_block(from: u32, block: usize) -> To16k {
        let g = gcd(u64::from(from), u64::from(SAMPLE_RATE)) as usize;
        let (up, down) = (SAMPLE_RATE as usize / g, from as usize / g);
        // `audio::resample`'s reach: 32 zero crossings of a sinc at 0.45 of the lower
        // rate, in input samples, plus slack.
        let lower = f64::from(from.min(SAMPLE_RATE));
        let reach = (32.0 * f64::from(from) / (0.9 * lower)).ceil() as usize + 64;
        let round_up = |n: usize| n.div_ceil(down).max(1) * down;
        To16k {
            from,
            up,
            down,
            margin: round_up(reach),
            block: round_up(block),
            input: Vec::new(),
            start: 0,
            next: 0,
            out: Vec::new(),
        }
    }

    fn push(&mut self, samples: &[f32]) {
        if self.from == SAMPLE_RATE {
            self.out.extend(f32_to_s16(samples));
            return;
        }
        self.input.extend_from_slice(samples);
        while self.start + self.input.len() >= self.next + self.block + self.margin {
            self.emit(self.block);
        }
    }

    /// The output owed for the input `[next, next + n)`, with `n` a multiple of `down`.
    fn emit(&mut self, n: usize) {
        let from = self.next.saturating_sub(self.margin);
        let to = (self.next + n + self.margin).min(self.start + self.input.len());
        let resampled = resample_to_16k(
            self.input
                .get(from - self.start..to - self.start)
                .unwrap_or_default(),
            self.from,
        );
        let skip = (self.next - from) * self.up / self.down;
        let take = n * self.up / self.down;
        self.out.extend(f32_to_s16(
            resampled.get(skip..skip + take).unwrap_or_default(),
        ));
        self.next += n;
        let keep = self.next.saturating_sub(self.margin);
        if keep > self.start {
            self.input.drain(..keep - self.start);
            self.start = keep;
        }
    }

    fn finish(mut self) -> Vec<i16> {
        let end = self.start + self.input.len();
        if self.from != SAMPLE_RATE && end > self.next {
            let from = self.next.saturating_sub(self.margin);
            let resampled = resample_to_16k(
                self.input.get(from - self.start..).unwrap_or_default(),
                self.from,
            );
            let skip = (self.next - from) * self.up / self.down;
            self.out
                .extend(f32_to_s16(resampled.get(skip..).unwrap_or_default()));
        }
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A 440 Hz tone at `rate`, `seconds` long.
    fn tone(rate: u32, seconds: f64) -> Vec<f32> {
        let n = (f64::from(rate) * seconds).round() as usize;
        (0..n)
            .map(|i| {
                (0.3 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / f64::from(rate)).sin())
                    as f32
            })
            .collect()
    }

    /// A RIFF WAVE file: 16 bit integer (`float == false`) or 32 bit float samples,
    /// `channels` interleaved copies of `mono`.
    fn wav(rate: u32, channels: u16, float: bool, mono: &[f32]) -> Vec<u8> {
        let bits: u16 = if float { 32 } else { 16 };
        let block_align = channels * bits / 8;
        let mut data = Vec::new();
        for &x in mono {
            for _ in 0..channels {
                if float {
                    data.extend_from_slice(&x.to_le_bytes());
                } else {
                    let s = (f64::from(x) * 32_767.0).round() as i16;
                    data.extend_from_slice(&s.to_le_bytes());
                }
            }
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&(if float { 3u16 } else { 1u16 }).to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * u32::from(block_align)).to_le_bytes());
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    fn crc8(bytes: &[u8]) -> u8 {
        let mut crc = 0u8;
        for &b in bytes {
            crc ^= b;
            for _ in 0..8 {
                crc = if crc & 0x80 != 0 {
                    (crc << 1) ^ 0x07
                } else {
                    crc << 1
                };
            }
        }
        crc
    }

    fn crc16(bytes: &[u8]) -> u16 {
        let mut crc = 0u16;
        for &b in bytes {
            crc ^= u16::from(b) << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 != 0 {
                    (crc << 1) ^ 0x8005
                } else {
                    crc << 1
                };
            }
        }
        crc
    }

    /// FLAC's UTF-8 style frame number.
    fn utf8_number(n: u64) -> Vec<u8> {
        if n < 0x80 {
            return vec![n as u8];
        }
        let mut tail = Vec::new();
        let mut v = n;
        let mut bytes = 1;
        while v >= (1 << (6 - bytes)) {
            tail.push(0x80 | (v & 0x3F) as u8);
            v >>= 6;
            bytes += 1;
        }
        let lead = (0xFFu16 << (8 - bytes)) as u8 | v as u8;
        let mut out = vec![lead];
        out.extend(tail.iter().rev());
        out
    }

    /// A FLAC file written at test time with VERBATIM subframes, 16 bit, independent
    /// channels; no encoder is needed and every decoder must read it.
    fn flac(rate: u32, channels: u8, mono: &[f32]) -> Vec<u8> {
        const BLOCK: usize = 4_096;
        let samples: Vec<i16> = mono
            .iter()
            .map(|&x| (f64::from(x) * 32_767.0).round() as i16)
            .collect();
        let mut out = b"fLaC".to_vec();
        out.extend_from_slice(&[0x80, 0, 0, 34]); // last block, STREAMINFO, 34 bytes
        out.extend_from_slice(&(BLOCK as u16).to_be_bytes());
        out.extend_from_slice(&(BLOCK as u16).to_be_bytes());
        out.extend_from_slice(&[0; 6]); // frame sizes unknown
        let total = samples.len() as u64;
        let packed: u64 =
            (u64::from(rate) << 44) | (u64::from(channels - 1) << 41) | (15u64 << 36) | total;
        out.extend_from_slice(&packed.to_be_bytes());
        out.extend_from_slice(&[0; 16]); // no MD5
        for (index, chunk) in samples.chunks(BLOCK).enumerate() {
            let mut frame = vec![0xFF, 0xF8];
            let size_code: u8 = if chunk.len() == BLOCK { 0b1100 } else { 0b0111 };
            // Low four bits 0000: the sample rate is STREAMINFO's.
            frame.push(size_code << 4);
            frame.push(((channels - 1) << 4) | (0b100 << 1));
            frame.extend(utf8_number(index as u64));
            if size_code == 0b0111 {
                frame.extend_from_slice(&((chunk.len() - 1) as u16).to_be_bytes());
            }
            frame.push(crc8(&frame));
            // Every channel the same VERBATIM subframe, with no wasted bits.
            let subframe: Vec<u8> = std::iter::once(0b0000_0010u8)
                .chain(chunk.iter().flat_map(|s| s.to_be_bytes()))
                .collect();
            for _ in 0..channels {
                frame.extend_from_slice(&subframe);
            }
            let crc = crc16(&frame);
            frame.extend_from_slice(&crc.to_be_bytes());
            out.extend(frame);
        }
        out
    }

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(bytes))
            .unwrap();
        path
    }

    fn assert_length(decoded: &Decoded, seconds: f64) {
        let expected = 16_000.0 * seconds;
        let got = decoded.samples.len() as f64;
        assert!(
            (got - expected).abs() <= expected * 0.01,
            "{got} samples, expected {expected}"
        );
    }

    #[test]
    fn wav_of_any_rate_depth_and_channels_becomes_16_khz_mono() {
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            ("stereo-44k.wav", 44_100, 2, false, 3.0),
            ("float-48k.wav", 48_000, 1, true, 2.0),
            ("mono-8k.wav", 8_000, 1, false, 2.5),
            ("mono-16k.wav", 16_000, 1, false, 1.0),
        ];
        for (name, rate, channels, float, seconds) in cases {
            let path = write(
                dir.path(),
                name,
                &wav(rate, channels, float, &tone(rate, seconds)),
            );
            let decoded = decode_path(&path).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(
                (decoded.source_rate, decoded.channels),
                (rate, channels as usize)
            );
            assert_length(&decoded, seconds);
            // The tone survives at its level: 0.3 of full scale, about 9,800 at peak.
            let peak = decoded
                .samples
                .iter()
                .map(|s| s.unsigned_abs())
                .max()
                .unwrap();
            assert!((9_000..=10_500).contains(&peak), "{name}: peak {peak}");
        }
    }

    #[test]
    fn flac_becomes_16_khz_mono() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "tone.flac",
            &flac(22_050, 2, &tone(22_050, 2.0)),
        );
        let decoded = decode_path(&path).expect("FLAC");
        assert_eq!((decoded.source_rate, decoded.channels), (22_050, 2));
        assert_length(&decoded, 2.0);
        // From standard input, with no name to go by.
        let bytes = std::fs::read(&path).unwrap();
        let piped = decode_reader(bytes.as_slice()).expect("FLAC on stdin");
        assert_eq!(piped, decoded);
    }

    #[test]
    fn opus_and_webm_are_refused_with_the_ffmpeg_line() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = wav(16_000, 1, false, &tone(16_000, 1.0));
        for (name, kind) in [
            ("memo.opus", "Opus"),
            ("talk.webm", "WebM"),
            ("a.WEBA", "WebM"),
            ("voice memo.m4a", "M4A and AAC"),
            ("clip.aac", "M4A and AAC"),
            ("song.ogg", "Ogg"),
        ] {
            let path = write(dir.path(), name, &bytes);
            let error = decode_path(&path).unwrap_err();
            assert_eq!(error, refused(kind));
            assert_eq!(error.outcome(), Outcome::UnsupportedFormat);
            assert!(
                error.message(name).contains("ffmpeg -i"),
                "{}",
                error.message(name)
            );
        }
        // By their bytes, when nothing names them.
        let mut ebml = vec![0x1A, 0x45, 0xDF, 0xA3];
        ebml.extend_from_slice(&[0; 60]);
        assert_eq!(decode_reader(ebml.as_slice()).unwrap_err(), refused("WebM"));
        let mut ogg = b"OggS".to_vec();
        ogg.extend_from_slice(&[0; 24]);
        ogg.extend_from_slice(b"OpusHead");
        ogg.extend_from_slice(&[0; 20]);
        let path = write(dir.path(), "voice.ogg", &ogg);
        assert_eq!(decode_path(&path).unwrap_err(), refused("Opus"));
        let mut mp4 = vec![0, 0, 0, 32];
        mp4.extend_from_slice(b"ftypM4A ");
        mp4.extend_from_slice(&[0; 52]);
        assert_eq!(
            decode_reader(mp4.as_slice()).unwrap_err(),
            refused("M4A and AAC")
        );
        // Bytes no reader knows: named by the file's extension, or not named at all.
        let path = write(dir.path(), "notes.txt", b"these are notes, not audio");
        assert_eq!(decode_path(&path).unwrap_err(), refused(".txt"));
        // A WAV that holds a codec Biorouter does not decode (IMA ADPCM, as some voice
        // recorders write) names no kind: Biorouter reads WAV, so ".wav" is not what was
        // refused. Its example writes a new name, never over the file it reads [M: the
        // `memo.wav` line ffmpeg was given for `memo.wav` exited with "already exists"].
        let mut adpcm = wav(16_000, 1, false, &tone(16_000, 1.0));
        adpcm[20..22].copy_from_slice(&0x0011u16.to_le_bytes());
        let path = write(dir.path(), "REC001.WAV", &adpcm);
        let error = decode_path(&path).unwrap_err();
        assert_eq!(error, DecodeError::UnsupportedFormat { kind: None });
        assert_eq!(
            error.message("REC001.WAV"),
            "Biorouter cannot read this recording. Convert it to WAV first, for example: ffmpeg -i REC001.WAV -ar 16000 -ac 1 REC001-16k.wav"
        );
        // An M4A renamed `.wav` is named by its bytes, and its example writes a new name too.
        let mut renamed = vec![0, 0, 0, 32];
        renamed.extend_from_slice(b"ftypM4A ");
        renamed.extend_from_slice(&[0; 52]);
        let path = write(dir.path(), "memo.wav", &renamed);
        let error = decode_path(&path).unwrap_err();
        assert_eq!(error, refused("M4A and AAC"));
        assert!(
            error
                .message("memo.wav")
                .ends_with("ffmpeg -i memo.wav -ar 16000 -ac 1 memo-16k.wav"),
            "{}",
            error.message("memo.wav")
        );
        let unnamed = decode_reader(b"these are notes, not audio".as_slice()).unwrap_err();
        assert_eq!(unnamed, DecodeError::UnsupportedFormat { kind: None });
        assert!(unnamed
            .message("-")
            .starts_with("Biorouter cannot read this recording."));
    }

    #[test]
    fn too_long_and_too_short_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        // Standard input over 200 MB is refused, since it is held whole.
        assert_eq!(
            decode_reader(std::io::repeat(0).take(MAX_INPUT_BYTES + 1)).unwrap_err(),
            DecodeError::TooLong
        );
        // A file is streamed, so its size alone refuses nothing: one second of 16 kHz WAV
        // followed by more than 200 MB of other bytes (a sparse file) decodes to that
        // second, as a 44.1 kHz stereo recorder's 20 minutes would.
        let big = write(
            dir.path(),
            "big.wav",
            &wav(16_000, 1, false, &tone(16_000, 1.0)),
        );
        std::fs::OpenOptions::new()
            .write(true)
            .open(&big)
            .and_then(|f| f.set_len(MAX_INPUT_BYTES + 1))
            .unwrap();
        assert_length(&decode_path(&big).expect("a large file"), 1.0);
        // A header that declares over 2 hours: 8 kHz, 8 bit, 7,201 s.
        let mut header = wav(8_000, 1, false, &[]);
        // 8 bit samples: bits per sample at 34, block align at 32, byte rate at 28.
        header[34..36].copy_from_slice(&8u16.to_le_bytes());
        header[32..34].copy_from_slice(&1u16.to_le_bytes());
        header[28..32].copy_from_slice(&8_000u32.to_le_bytes());
        let declared = 7_201u32 * 8_000;
        header[40..44].copy_from_slice(&declared.to_le_bytes());
        header[4..8].copy_from_slice(&(36 + declared).to_le_bytes());
        header.extend(vec![128u8; 16_000]);
        let path = write(dir.path(), "long.wav", &header);
        assert_eq!(decode_path(&path).unwrap_err(), DecodeError::TooLong);
        // Under half a second.
        let path = write(
            dir.path(),
            "short.wav",
            &wav(16_000, 1, false, &tone(16_000, 0.4)),
        );
        let decoded = decode_path(&path).expect("decodes");
        assert_eq!(decoded.check_length(), Err(DecodeError::TooShort));
        assert_eq!(DecodeError::TooShort.outcome().exit_code(), 3);
        let path = write(
            dir.path(),
            "half.wav",
            &wav(16_000, 1, false, &tone(16_000, 0.5)),
        );
        assert_eq!(decode_path(&path).unwrap().check_length(), Ok(()));
        assert_eq!(
            decode_path(&dir.path().join("missing.wav")).unwrap_err(),
            DecodeError::NotFound
        );
    }

    #[test]
    fn digital_silence_is_found_without_the_engine() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "zeros.wav",
            &wav(44_100, 2, false, &[0.0; 44_100]),
        );
        let decoded = decode_path(&path).unwrap();
        assert!(decoded.is_digital_silence());
        let path = write(
            dir.path(),
            "tone.wav",
            &wav(16_000, 1, false, &tone(16_000, 1.0)),
        );
        assert!(!decode_path(&path).unwrap().is_digital_silence());
    }

    /// The block resampler gives exactly what one resampling of the whole recording
    /// gives, at every common rate, with blocks far smaller than in use.
    #[test]
    fn block_resampling_equals_whole_resampling() {
        for rate in [8_000u32, 11_025, 22_050, 44_100, 48_000, 96_000] {
            let input = tone(rate, 3.3);
            let whole = f32_to_s16(&resample_to_16k(&input, rate));
            let mut blocks = To16k::with_block(rate, rate as usize / 3);
            for piece in input.chunks(1_157) {
                blocks.push(piece);
            }
            assert_eq!(blocks.finish(), whole, "{rate} Hz");
        }
    }
}
