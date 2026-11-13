//! Reading a recording for `biorouter transcribe` (DD-78, O-39).
//!
//! symphonia decodes, in this crate only: WAV of any rate, channel count and depth
//! (integer, float, A-law and mu-law), AIFF, FLAC, MP3, AAC in an MP4 file (`.m4a`,
//! `.mp4`, a phone's or a Mac's voice memo) or in ADTS (`.aac`, including the MPEG-2
//! headers Apple's encoder writes), Apple Lossless in an M4A (a voice memo recorded with
//! Lossless on, O-41), and Ogg holding Vorbis or FLAC. An M4A's encoder
//! priming and padding are dropped as its edit list or Apple's `iTunSMPB` tag says
//! ([`Trim`]), which symphonia 0.5 reads but does not apply; ADTS says nothing of them, so
//! a `.aac` keeps up to 48 ms of priming before its speech. The engine and the daemon take
//! 16 kHz mono PCM and never parse a file. Opus and WebM are refused before anything is
//! decoded, with an `ffmpeg` line that converts them, and so is a codec found inside a
//! container symphonia reads but cannot decode, named when it is Opus.
//!
//! The decode runs on its own thread while the model loads (`commands/transcribe.rs`), so
//! a compressed format adds no wait before the first decode unless reading the file takes
//! longer than the load: 1 to 3 ms for a dictation of seconds in any format, and about 1 s
//! for a 13.6 minute 48 kHz M4A (36 ms for the same recording as 16 kHz WAV), most of it
//! the resampling a 16 kHz WAV does not need [M, release, 2026-11-07].
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
use symphonia::core::codecs::{
    CodecType, Decoder, DecoderOptions, CODEC_TYPE_NULL, CODEC_TYPE_OPUS,
};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader};
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
    /// `WebM`, or the file's extension); `None` for bytes no reader
    /// knows that came with no name, or a file named as a kind Biorouter reads.
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
/// something else (an ADPCM WAV from a voice recorder, compressed AIFF-C, an Opus stream in
/// an MP4, a WebM renamed), so its extension does not name what was refused, and the message
/// names no kind unless the codec does.
const READ_EXTENSIONS: [&str; 13] = [
    "wav", "wave", "aif", "aiff", "aifc", "flac", "mp3", "m4a", "mp4", "m4b", "aac", "ogg", "oga",
];

/// The kinds Biorouter does not decode, by the name of the file.
fn refused_extension(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "opus" => Some("Opus"),
        "webm" | "weba" => Some("WebM"),
        _ => None,
    }
}

/// The same kinds by their first bytes, for standard input or a misnamed file:
/// Matroska's EBML magic, and an Ogg page whose first packet is an Opus header (an Ogg
/// page of Vorbis or FLAC is read).
fn refused_magic(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some("WebM");
    }
    if head.starts_with(b"OggS") && head.windows(8).any(|w| w == b"OpusHead") {
        return Some("Opus");
    }
    None
}

/// The name of a codec that a container Biorouter reads can hold but no decoder here
/// takes: Opus in an MP4 or Ogg file. Apple Lossless in an M4A is read (O-41).
fn refused_codec(codec: CodecType) -> Option<&'static str> {
    match codec {
        CODEC_TYPE_OPUS => Some("Opus"),
        _ => None,
    }
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
    let head = head.get(..got).unwrap_or_default();
    if is_mpeg2_adts(head) {
        let source = Mpeg2Adts::new(file)?;
        return decode(Box::new(source), ext.as_deref(), kind, Trim::NONE);
    }
    let trim = if is_mp4(head) {
        let trim = mp4_trim(&mut file).unwrap_or(Trim::NONE);
        file.seek(SeekFrom::Start(0))
            .map_err(|_| DecodeError::Unreadable)?;
        trim
    } else {
        Trim::NONE
    };
    decode(Box::new(file), ext.as_deref(), kind, trim)
}

/// Whether `head` starts an MP4 file (`ftyp` as its first box): M4A, MP4, M4B.
fn is_mp4(head: &[u8]) -> bool {
    head.get(4..8) == Some(b"ftyp".as_slice())
}

/// What an MP4 file says to drop from its decoded audio, in frames of `timescale`: the
/// encoder's priming at the start (`skip`) and, through `keep`, the padding at the end.
/// symphonia 0.5's MP4 reader parses the edit list that says so but does not apply it,
/// so AAC from an M4A came out with Apple's 2,112 or ffmpeg's 1,024 priming samples first
/// and its speech up to 48 ms late, which moved the decoder's cuts: a 13.6 minute M4A
/// (Apple's encoder, 48 kHz) transcribed at a word error rate of 8.3 % against its WAV,
/// and at 2.3 % with the priming dropped, word for word what ffmpeg's decode of the same
/// file gives [M, small.en, 2026-11-07].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Trim {
    skip: u64,
    keep: Option<u64>,
    /// The units of `skip` and `keep`, per second; 0 means they are frames already.
    timescale: u64,
}

impl Trim {
    const NONE: Trim = Trim {
        skip: 0,
        keep: None,
        timescale: 0,
    };

    /// `skip` and `keep` in frames at `rate`. A file's values can be anything up to
    /// `u64::MAX`, so a product past it reads as `u64::MAX`, which keeps or skips all.
    fn at_rate(self, rate: u32) -> (u64, Option<u64>) {
        let frames = |v: u64| match self.timescale {
            0 => v,
            scale => u64::try_from(u128::from(v) * u128::from(rate) / u128::from(scale))
                .unwrap_or(u64::MAX),
        };
        (frames(self.skip), self.keep.map(frames))
    }
}

/// The most of an MP4 file's `moov` box this reads to find its trim: a voice memo's is
/// about 160 KB for 13 minutes; past this the audio is decoded untrimmed.
const MAX_MOOV_BYTES: u64 = 16 << 20;

/// The trim of an MP4 file: its sound track's edit list (`elst`: the first edit's
/// `media_time` in the track's timescale, and its `segment_duration` in the movie's), as
/// ffmpeg writes it, or else Apple's `iTunSMPB` tag (priming, padding and the original
/// length in samples), as Apple's encoder writes it. `None` when the file has neither or
/// cannot be read that far; the audio is then decoded whole.
fn mp4_trim(source: &mut (impl Read + Seek)) -> Option<Trim> {
    let len = source.seek(SeekFrom::End(0)).ok()?;
    let (start, size) = find_box(source, 0, len, b"moov")?;
    if size > MAX_MOOV_BYTES {
        return None;
    }
    source.seek(SeekFrom::Start(start)).ok()?;
    let mut moov = vec![0u8; size as usize];
    source.read_exact(&mut moov).ok()?;
    let movie_scale = boxes(&moov)
        .find(|(kind, _)| kind == b"mvhd")
        .and_then(|(_, body)| timescale(body))?;
    for (_, trak) in boxes(&moov).filter(|(kind, _)| kind == b"trak") {
        let Some(mdia) = child(trak, b"mdia") else {
            continue;
        };
        let sound = child(mdia, b"hdlr").and_then(|h| h.get(8..12)) == Some(b"soun".as_slice());
        if !sound {
            continue;
        }
        let Some(media_scale) = child(mdia, b"mdhd").and_then(timescale) else {
            break;
        };
        if let Some((duration, media_time)) = child(trak, b"edts")
            .and_then(|edts| child(edts, b"elst"))
            .and_then(first_edit)
        {
            if media_time > 0 {
                // The edit's duration is in the movie's timescale, its start in the track's.
                let keep = u128::from(duration) * u128::from(media_scale) / u128::from(movie_scale);
                return Some(Trim {
                    skip: media_time,
                    keep: (keep > 0).then(|| u64::try_from(keep).unwrap_or(u64::MAX)),
                    timescale: media_scale,
                });
            }
        }
        break;
    }
    itunsmpb(&moov)
}

/// The boxes of `bytes`, each as its type and body, stopping at the first that does not
/// fit.
fn boxes(bytes: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut at = 0usize;
    std::iter::from_fn(move || {
        let head = bytes.get(at..at + 8)?;
        let mut size = u32::from_be_bytes(head.get(..4)?.try_into().ok()?) as usize;
        let kind: [u8; 4] = head.get(4..8)?.try_into().ok()?;
        let mut header = 8;
        if size == 1 {
            size = u64::from_be_bytes(bytes.get(at + 8..at + 16)?.try_into().ok()?) as usize;
            header = 16;
        } else if size == 0 {
            size = bytes.len() - at;
        }
        let body = bytes.get(at + header..at.checked_add(size)?)?;
        at += size.max(header);
        Some((kind, body))
    })
}

fn child<'a>(bytes: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(bytes).find(|(k, _)| k == kind).map(|(_, body)| body)
}

/// The timescale of an `mvhd` or `mdhd` body, version 0 or 1.
fn timescale(body: &[u8]) -> Option<u64> {
    let at = if *body.first()? == 1 { 20 } else { 12 };
    let scale = u32::from_be_bytes(body.get(at..at + 4)?.try_into().ok()?);
    (scale > 0).then_some(u64::from(scale))
}

/// The first edit of an `elst` body that is not an empty edit: its segment duration and
/// media time.
fn first_edit(body: &[u8]) -> Option<(u64, u64)> {
    let version = *body.first()?;
    let count = u32::from_be_bytes(body.get(4..8)?.try_into().ok()?) as usize;
    let entry = if version == 1 { 20 } else { 12 };
    for i in 0..count.min(16) {
        let e = body.get(8 + i * entry..8 + (i + 1) * entry)?;
        let (duration, media_time) = if version == 1 {
            (
                u64::from_be_bytes(e.get(..8)?.try_into().ok()?),
                i64::from_be_bytes(e.get(8..16)?.try_into().ok()?),
            )
        } else {
            (
                u64::from(u32::from_be_bytes(e.get(..4)?.try_into().ok()?)),
                i64::from(i32::from_be_bytes(e.get(4..8)?.try_into().ok()?)),
            )
        };
        if media_time >= 0 {
            return Some((duration, media_time as u64));
        }
    }
    None
}

/// Apple's `iTunSMPB` tag in `moov/udta/meta/ilst`: " 00000000 <priming> <padding>
/// <original length> ..." in hexadecimal samples.
fn itunsmpb(moov: &[u8]) -> Option<Trim> {
    let meta = child(child(moov, b"udta")?, b"meta")?;
    // `meta` is a full box: four bytes of version and flags before its children.
    let ilst = child(meta.get(4..)?, b"ilst")?;
    for (_, item) in boxes(ilst).filter(|(kind, _)| kind == b"----") {
        let named = child(item, b"name").and_then(|n| n.get(4..)) == Some(b"iTunSMPB".as_slice());
        if !named {
            continue;
        }
        let text = std::str::from_utf8(child(item, b"data")?.get(8..)?).ok()?;
        let fields: Vec<u64> = text
            .split_whitespace()
            .map(|f| u64::from_str_radix(f, 16))
            .collect::<Result<_, _>>()
            .ok()?;
        let (priming, length) = (*fields.get(1)?, *fields.get(3)?);
        return (priming > 0 || length > 0).then_some(Trim {
            skip: priming,
            keep: (length > 0).then_some(length),
            timescale: 0,
        });
    }
    None
}

/// The first top level box of `kind` in `source` between `from` and `to`: its body's
/// offset and length. Walks the box headers only, so an `mdat` before `moov` is skipped.
fn find_box(
    source: &mut (impl Read + Seek),
    from: u64,
    to: u64,
    kind: &[u8; 4],
) -> Option<(u64, u64)> {
    let mut at = from;
    while at + 8 <= to {
        source.seek(SeekFrom::Start(at)).ok()?;
        let mut head = [0u8; 16];
        source.read_exact(head.get_mut(..8)?).ok()?;
        let mut size = u64::from(u32::from_be_bytes(head.get(..4)?.try_into().ok()?));
        let mut header = 8;
        if size == 1 {
            source.read_exact(head.get_mut(8..16)?).ok()?;
            size = u64::from_be_bytes(head.get(8..16)?.try_into().ok()?);
            header = 16;
        } else if size == 0 {
            size = to - at;
        }
        if size < header {
            return None;
        }
        if head.get(4..8)? == kind.as_slice() {
            return Some((at + header, size - header));
        }
        // A 64 bit size can carry `at` past `u64::MAX`, which would wrap it back to a box
        // already walked and loop for ever.
        at = at.checked_add(size)?;
    }
    None
}

/// Whether `head` starts an ADTS frame whose header names MPEG-2 and carries no CRC
/// (sync word 0xFFF9), as Apple's encoder writes `.aac` (afconvert, a Mac's export).
/// symphonia 0.5's ADTS reader finds frames by the MPEG-4 word (0xFFF1) alone, and the
/// AAC inside is the same, so such a file is read with the version bit changed
/// ([`Mpeg2Adts`], [`patch_mpeg2_adts`]). A header with a CRC (0xFFF8, 0xFFF0) is left as
/// it is: symphonia reads none.
fn is_mpeg2_adts(head: &[u8]) -> bool {
    head.starts_with(&[0xFF, 0xF9])
}

/// The length of the ADTS frame whose 7 byte header is `header`, when it is one: the sync
/// word, layer 0, and a length that holds at least the header.
fn adts_frame_len(header: &[u8]) -> Option<u64> {
    let [0xFF, b1, _, b3, b4, b5, _] = *header else {
        return None;
    };
    if b1 & 0xF6 != 0xF0 {
        return None;
    }
    let len = (u64::from(b3 & 0x03) << 11) | (u64::from(b4) << 3) | (u64::from(b5) >> 5);
    (len >= 7).then_some(len)
}

/// AAC's sampling rates, by an ADTS header's sampling frequency index.
const ADTS_RATES: [u32; 13] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350,
];

/// What the ADTS frame whose 7 byte header is `header` holds: its length in bytes, and its
/// audio as (samples, rate): 1,024 samples per raw data block at the header's rate (the
/// core rate, so HE-AAC's doubled output lasts as long). `None` when it is not a frame.
fn adts_frame(header: &[u8]) -> Option<(u64, u64, u32)> {
    let len = adts_frame_len(header)?;
    let rate = *ADTS_RATES.get(usize::from((*header.get(2)? >> 2) & 0x0F))?;
    let blocks = u64::from(*header.get(6)? & 0x03) + 1;
    Some((len, 1_024 * blocks, rate))
}

/// The offsets of the MPEG-2 ADTS frame headers in `source`, walking frame by frame from
/// its start until the bytes stop being frames (a trailing tag, or the end). The frames'
/// audio is added up as they are walked, and a stream past 2 hours is refused there, so
/// the list never outgrows 2 hours of frames whatever the file holds: at most 675,000
/// offsets (5.4 MB), for frames of 1,024 samples at 96 kHz.
fn mpeg2_adts_headers(source: &mut (impl Read + Seek)) -> Result<Vec<u64>, DecodeError> {
    let unreadable = |_| DecodeError::Unreadable;
    source.seek(SeekFrom::Start(0)).map_err(unreadable)?;
    let mut reader = std::io::BufReader::with_capacity(64 * 1024, source);
    let mut headers = Vec::new();
    let mut at = 0u64;
    let mut header = [0u8; 7];
    // The audio walked so far, in ticks of 1/28,224,000 s, the least common multiple of
    // every AAC rate, so the sum is exact.
    const TICKS: u64 = 28_224_000;
    let mut ticks = 0u64;
    while read_up_to(&mut reader, &mut header).map_err(unreadable)? == header.len() {
        let Some((len, samples, rate)) = adts_frame(&header) else {
            break;
        };
        ticks += samples * (TICKS / u64::from(rate));
        if ticks > MAX_SECONDS * TICKS {
            return Err(DecodeError::TooLong);
        }
        if header.get(1) == Some(&0xF9) {
            headers.push(at);
        }
        reader
            .seek_relative(len as i64 - header.len() as i64)
            .map_err(unreadable)?;
        at += len;
    }
    Ok(headers)
}

/// `bytes`, an ADTS stream held whole (standard input), with every MPEG-2 frame header
/// given the MPEG-4 bit, so symphonia reads it.
fn patch_mpeg2_adts(bytes: &mut [u8]) {
    let mut at = 0usize;
    while let Some(len) = bytes.get(at..at + 7).and_then(adts_frame_len) {
        if let Some(b1) = bytes.get_mut(at + 1).filter(|b| **b == 0xF9) {
            *b1 = 0xF1;
        }
        at += len as usize;
    }
}

/// A file of MPEG-2 ADTS frames, read as MPEG-4 ADTS: byte 1 of each frame header, found
/// by one pass over the headers before the decode, reads 0xF1 rather than 0xF9. Streamed
/// from disk like every other file; the pass reads 7 bytes per frame through a buffer.
struct Mpeg2Adts {
    file: std::fs::File,
    pos: u64,
    len: u64,
    /// Sorted offsets of the frame headers to patch.
    headers: Vec<u64>,
}

impl Mpeg2Adts {
    /// `TooLong` when its frames hold more than 2 hours.
    fn new(mut file: std::fs::File) -> Result<Mpeg2Adts, DecodeError> {
        let headers = mpeg2_adts_headers(&mut file)?;
        let unreadable = |_| DecodeError::Unreadable;
        let len = file.seek(SeekFrom::End(0)).map_err(unreadable)?;
        file.seek(SeekFrom::Start(0)).map_err(unreadable)?;
        Ok(Mpeg2Adts {
            file,
            pos: 0,
            len,
            headers,
        })
    }
}

impl Read for Mpeg2Adts {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.file.read(buf)?;
        let (start, end) = (self.pos, self.pos + n as u64);
        // The headers whose version byte (offset + 1) falls inside what was just read.
        let first = self.headers.partition_point(|&h| h + 1 < start);
        for &h in self.headers.get(first..).unwrap_or_default() {
            if h + 1 >= end {
                break;
            }
            if let Some(b) = buf.get_mut((h + 1 - start) as usize) {
                *b = 0xF1;
            }
        }
        self.pos = end;
        Ok(n)
    }
}

impl Seek for Mpeg2Adts {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        self.pos = self.file.seek(to)?;
        Ok(self.pos)
    }
}

impl MediaSource for Mpeg2Adts {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
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
    if is_mpeg2_adts(&bytes) {
        patch_mpeg2_adts(&mut bytes);
    }
    let mut source = std::io::Cursor::new(bytes);
    let trim = if is_mp4(source.get_ref()) {
        mp4_trim(&mut source).unwrap_or(Trim::NONE)
    } else {
        Trim::NONE
    };
    source.set_position(0);
    decode(Box::new(source), None, None, trim)
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

/// A probed container, the id of the track to decode, and that track's decoder.
type OpenTrack = (Box<dyn FormatReader>, u32, Box<dyn Decoder>);

/// The probed container, its first track with a codec, and that track's decoder. A
/// container or codec Biorouter does not read is refused (named by `kind`, or by the codec
/// when it is Opus), and a header that gives more than 2 hours is refused
/// before anything is decoded.
fn open_track(
    source: Box<dyn MediaSource>,
    ext: Option<&str>,
    kind: &Option<String>,
) -> Result<OpenTrack, DecodeError> {
    let unsupported = || DecodeError::UnsupportedFormat { kind: kind.clone() };
    let stream = MediaSourceStream::new(source, Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = ext {
        hint.with_extension(ext);
    }
    // Gapless: the encoder's delay and padding are trimmed, so the audio is as long as
    // what was recorded. symphonia does it for MP3 (the LAME header) and Ogg (granule
    // positions); an M4A's comes from `trim`, read from the file by `mp4_trim`.
    let options = FormatOptions {
        enable_gapless: true,
        ..FormatOptions::default()
    };
    let probed = symphonia::default::get_probe()
        .format(&hint, stream, &options, &MetadataOptions::default())
        .map_err(|_| unsupported())?;
    let format = probed.format;
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
    let decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .map_err(|_| match refused_codec(params.codec) {
            Some(codec) => refused(codec),
            None => unsupported(),
        })?;
    Ok((format, track_id, decoder))
}

/// Decodes `source`, refusing a damaged recording rather than crashing on it: symphonia
/// 0.5's readers can panic on bytes they do not check (its MP4 reader indexes a sample
/// table before checking it, `atoms/stsc.rs`), which a cut or corrupted `.m4a` reaches, so
/// a panic in here answers as any recording Biorouter cannot read does.
fn decode(
    source: Box<dyn MediaSource>,
    ext: Option<&str>,
    kind: Option<String>,
    trim: Trim,
) -> Result<Decoded, DecodeError> {
    let refused = DecodeError::UnsupportedFormat { kind: kind.clone() };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decode_unguarded(source, ext, kind, trim)
    }))
    .unwrap_or(Err(refused))
}

fn decode_unguarded(
    source: Box<dyn MediaSource>,
    ext: Option<&str>,
    kind: Option<String>,
    trim: Trim,
) -> Result<Decoded, DecodeError> {
    let unsupported = || DecodeError::UnsupportedFormat { kind: kind.clone() };
    let (mut format, track_id, mut decoder) = open_track(source, ext, &kind)?;

    let mut resampler: Option<To16k> = None;
    let mut channels = 0usize;
    let mut frames_in: u64 = 0;
    // Every frame the decoder gave, priming and padding included.
    let mut decoded: u64 = 0;
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
        // The part of this packet's frames that `trim` keeps, at this stream's rate.
        let (skip, keep) = trim.at_rate(rate);
        let len = mono.len() as u64;
        let from = skip.saturating_sub(decoded).min(len);
        let to = keep.map_or(len, |keep| {
            skip.saturating_add(keep).saturating_sub(decoded).min(len)
        });
        decoded += len;
        let kept = mono
            .get(from as usize..to.max(from) as usize)
            .unwrap_or_default();
        frames_in += kept.len() as u64;
        if frames_in > MAX_SECONDS * u64::from(rate) {
            return Err(DecodeError::TooLong);
        }
        to16k.push(kept);
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
        let (stream_info, frames) = flac_parts(rate, channels, mono);
        let mut out = b"fLaC".to_vec();
        out.extend(stream_info);
        for (_, frame) in frames {
            out.extend(frame);
        }
        out
    }

    /// The same FLAC stream in Ogg pages (the Ogg FLAC mapping, version 1.0): the
    /// identification packet with STREAMINFO alone on the first page, then one frame per
    /// page, so the Ogg reader is tested with no encoder.
    fn ogg_flac(rate: u32, channels: u8, mono: &[f32]) -> Vec<u8> {
        let (stream_info, frames) = flac_parts(rate, channels, mono);
        let mut first = vec![0x7F];
        first.extend_from_slice(b"FLAC");
        first.extend_from_slice(&[1, 0, 0, 0]); // mapping 1.0, header count unknown
        first.extend_from_slice(b"fLaC");
        first.extend(stream_info);
        let mut out = ogg_page(0, 0, 0x02, &first);
        let mut granule = 0u64;
        let last = frames.len();
        for (index, (samples, frame)) in frames.into_iter().enumerate() {
            granule += samples as u64;
            let flags = if index + 1 == last { 0x04 } else { 0 };
            out.extend(ogg_page(index as u32 + 1, granule, flags, &frame));
        }
        out
    }

    /// One Ogg page holding one whole packet.
    fn ogg_page(sequence: u32, granule: u64, flags: u8, packet: &[u8]) -> Vec<u8> {
        let mut lacing = vec![255u8; packet.len() / 255];
        lacing.push((packet.len() % 255) as u8);
        assert!(lacing.len() <= 255, "one packet per page");
        let mut page = b"OggS".to_vec();
        page.push(0);
        page.push(flags);
        page.extend_from_slice(&granule.to_le_bytes());
        page.extend_from_slice(&0x4272_6F75u32.to_le_bytes());
        page.extend_from_slice(&sequence.to_le_bytes());
        page.extend_from_slice(&[0; 4]);
        page.push(lacing.len() as u8);
        page.extend(lacing);
        page.extend_from_slice(packet);
        let mut crc = 0u32;
        for &b in &page {
            crc ^= u32::from(b) << 24;
            for _ in 0..8 {
                crc = if crc & 0x8000_0000 != 0 {
                    (crc << 1) ^ 0x04C1_1DB7
                } else {
                    crc << 1
                };
            }
        }
        page[22..26].copy_from_slice(&crc.to_le_bytes());
        page
    }

    /// FLAC's STREAMINFO block (with its header) and its frames, each with its length in
    /// samples.
    fn flac_parts(rate: u32, channels: u8, mono: &[f32]) -> (Vec<u8>, Vec<(usize, Vec<u8>)>) {
        const BLOCK: usize = 4_096;
        let samples: Vec<i16> = mono
            .iter()
            .map(|&x| (f64::from(x) * 32_767.0).round() as i16)
            .collect();
        let mut out = vec![0x80, 0, 0, 34]; // last block, STREAMINFO, 34 bytes
        out.extend_from_slice(&(BLOCK as u16).to_be_bytes());
        out.extend_from_slice(&(BLOCK as u16).to_be_bytes());
        out.extend_from_slice(&[0; 6]); // frame sizes unknown
        let total = samples.len() as u64;
        let packed: u64 =
            (u64::from(rate) << 44) | (u64::from(channels - 1) << 41) | (15u64 << 36) | total;
        out.extend_from_slice(&packed.to_be_bytes());
        out.extend_from_slice(&[0; 16]); // no MD5
        let mut frames = Vec::new();
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
            frames.push((chunk.len(), frame));
        }
        (out, frames)
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

    /// The Ogg reader (O-39): the same FLAC stream in Ogg pages gives the same samples, from
    /// a file and from standard input. Vorbis in Ogg, AAC in MP4 and ADTS need an encoder,
    /// so they are the committed files of [`real_m4a_aac_and_ogg_vorbis_decode_in_time`].
    #[test]
    fn ogg_becomes_16_khz_mono() {
        let dir = tempfile::tempdir().unwrap();
        let mono = tone(22_050, 2.0);
        let native =
            decode_path(&write(dir.path(), "tone.flac", &flac(22_050, 2, &mono))).expect("FLAC");
        for name in ["tone.ogg", "tone.oga", "renamed.wav"] {
            let path = write(dir.path(), name, &ogg_flac(22_050, 2, &mono));
            let decoded = decode_path(&path).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(decoded, native, "{name}");
        }
        let piped =
            decode_reader(ogg_flac(22_050, 2, &mono).as_slice()).expect("Ogg FLAC on stdin");
        assert_eq!(piped, native);
    }

    #[test]
    fn opus_and_webm_are_refused_with_the_ffmpeg_line() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = wav(16_000, 1, false, &tone(16_000, 1.0));
        for (name, kind) in [
            ("memo.opus", "Opus"),
            ("talk.webm", "WebM"),
            ("a.WEBA", "WebM"),
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
        assert_eq!(decode_reader(ogg.as_slice()).unwrap_err(), refused("Opus"));
        // M4A, AAC and Ogg are read now (O-39): a name that says so decides nothing, the
        // bytes do. A WAV named `.m4a`, `.aac` or `.ogg` is read as the WAV it is.
        for name in ["voice memo.m4a", "clip.aac", "song.ogg", "talk.mp4"] {
            let path = write(dir.path(), name, &bytes);
            let decoded = decode_path(&path).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_length(&decoded, 1.0);
        }
        // A damaged M4A or Ogg file names no kind: its name is one Biorouter reads.
        let mut mp4 = vec![0, 0, 0, 32];
        mp4.extend_from_slice(b"ftypM4A ");
        mp4.extend_from_slice(&[0; 52]);
        let unnamed = DecodeError::UnsupportedFormat { kind: None };
        assert_eq!(decode_reader(mp4.as_slice()).unwrap_err(), unnamed);
        let path = write(dir.path(), "cut.m4a", &mp4);
        let error = decode_path(&path).unwrap_err();
        assert_eq!(error, unnamed);
        assert_eq!(
            error.message("cut.m4a"),
            "Biorouter cannot read this recording. Convert it to WAV first, for example: ffmpeg -i cut.m4a -ar 16000 -ac 1 cut.wav"
        );
        let mut page = b"OggS".to_vec();
        page.extend_from_slice(&[0; 60]);
        let path = write(dir.path(), "cut.ogg", &page);
        assert_eq!(decode_path(&path).unwrap_err(), unnamed);
        assert_eq!(decode_reader(page.as_slice()).unwrap_err(), unnamed);
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
        // A WebM renamed `.wav` is named by its bytes, and its example writes a new name too.
        let path = write(dir.path(), "memo.wav", &ebml);
        let error = decode_path(&path).unwrap_err();
        assert_eq!(error, refused("WebM"));
        assert!(
            error
                .message("memo.wav")
                .ends_with("ffmpeg -i memo.wav -ar 16000 -ac 1 memo-16k.wav"),
            "{}",
            error.message("memo.wav")
        );
        assert_eq!(
            refused_codec(symphonia::core::codecs::CODEC_TYPE_ALAC),
            None,
            "Apple Lossless is read (O-41)"
        );
        assert_eq!(refused_codec(CODEC_TYPE_OPUS), Some("Opus"));
        assert_eq!(refused_codec(symphonia::core::codecs::CODEC_TYPE_AAC), None);
        let unnamed = decode_reader(b"these are notes, not audio".as_slice()).unwrap_err();
        assert_eq!(unnamed, DecodeError::UnsupportedFormat { kind: None });
        assert!(unnamed
            .message("-")
            .starts_with("Biorouter cannot read this recording."));
    }

    /// Apple's ADTS (sync 0xFFF9, MPEG-2) is read as MPEG-4 ADTS: byte 1 of every frame
    /// header becomes 0xF1, and nothing else changes, not a 0xF9 inside a payload, not a
    /// tag after the last frame, whatever the reads' sizes and a seek between them. The
    /// decode of a real one is in [`real_m4a_aac_and_ogg_vorbis_decode_in_time`].
    #[test]
    fn mpeg2_adts_headers_read_as_mpeg4() {
        let header = |len: usize, version: u8| -> Vec<u8> {
            vec![
                0xFF,
                version,
                0x60,
                0x40 | ((len >> 11) as u8 & 0x03),
                (len >> 3) as u8,
                ((len & 0x07) as u8) << 5 | 0x1F,
                0xFC,
            ]
        };
        let mut apple = Vec::new();
        let mut expected = Vec::new();
        for (i, payload_len) in [6usize, 300, 0, 2_100, 41].into_iter().enumerate() {
            let payload: Vec<u8> = (0..payload_len)
                .map(|j| if j % 7 == 1 { 0xF9 } else { (i + j) as u8 })
                .collect();
            apple.extend(header(7 + payload_len, 0xF9));
            apple.extend(&payload);
            expected.extend(header(7 + payload_len, 0xF1));
            expected.extend(&payload);
        }
        // An ID3 tag after the frames is not a frame: the walk stops there.
        apple.extend_from_slice(b"TAG\xFF\xF9 not a frame");
        expected.extend_from_slice(b"TAG\xFF\xF9 not a frame");
        assert!(is_mpeg2_adts(&apple) && !is_mpeg2_adts(&expected));

        let mut patched = apple.clone();
        patch_mpeg2_adts(&mut patched);
        assert_eq!(patched, expected);

        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "apple.aac", &apple);
        let mut source = Mpeg2Adts::new(std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(source.headers, [0, 13, 320, 327, 2_434]);
        assert_eq!(source.byte_len(), Some(apple.len() as u64));
        let mut read = Vec::new();
        let mut chunk = [0u8; 97];
        loop {
            let n = source.read(&mut chunk).unwrap();
            if n == 0 {
                break;
            }
            read.extend_from_slice(&chunk[..n]);
        }
        assert_eq!(read, expected);
        // A read after a seek patches by position too.
        source.seek(SeekFrom::Start(318)).unwrap();
        let mut window = [0u8; 12];
        source.read_exact(&mut window).unwrap();
        assert_eq!(&window[..], &expected[318..330]);
    }

    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    /// A version 0 `mvhd` or `mdhd` body with `scale`.
    fn header_body(scale: u32) -> Vec<u8> {
        let mut body = vec![0; 12];
        body.extend_from_slice(&scale.to_be_bytes());
        body.extend_from_slice(&[0; 8]);
        body
    }

    /// An MP4 file whose sound track's `moov` holds `edits` (segment duration, media time)
    /// and, when given, an `iTunSMPB` text, with `mdat` before `moov` as a phone writes it.
    fn mp4(
        movie_scale: u32,
        media_scale: u32,
        edits: &[(u32, i32)],
        smpb: Option<&str>,
    ) -> Vec<u8> {
        let mut elst = vec![0, 0, 0, 0];
        elst.extend_from_slice(&(edits.len() as u32).to_be_bytes());
        for (duration, media_time) in edits {
            elst.extend_from_slice(&duration.to_be_bytes());
            elst.extend_from_slice(&media_time.to_be_bytes());
            elst.extend_from_slice(&[0, 1, 0, 0]);
        }
        mp4_with(
            movie_scale,
            media_scale,
            (!edits.is_empty()).then_some(elst),
            smpb,
        )
    }

    /// The same file with `elst` as the edit list's whole body, of either version.
    fn mp4_with(
        movie_scale: u32,
        media_scale: u32,
        elst: Option<Vec<u8>>,
        smpb: Option<&str>,
    ) -> Vec<u8> {
        let mut hdlr = vec![0; 8];
        hdlr.extend_from_slice(b"soun");
        hdlr.extend_from_slice(&[0; 13]);
        let mut mdia = mp4_box(b"mdhd", &header_body(media_scale));
        mdia.extend(mp4_box(b"hdlr", &hdlr));
        let mut trak = mp4_box(b"tkhd", &[0; 84]);
        if let Some(elst) = elst {
            trak.extend(mp4_box(b"edts", &mp4_box(b"elst", &elst)));
        }
        trak.extend(mp4_box(b"mdia", &mdia));
        let mut moov = mp4_box(b"mvhd", &header_body(movie_scale));
        moov.extend(mp4_box(b"trak", &trak));
        if let Some(text) = smpb {
            let mut name = vec![0; 4];
            name.extend_from_slice(b"iTunSMPB");
            let mut data = vec![0, 0, 0, 1, 0, 0, 0, 0];
            data.extend_from_slice(text.as_bytes());
            let mut item = mp4_box(b"mean", b"\0\0\0\0com.apple.iTunes");
            item.extend(mp4_box(b"name", &name));
            item.extend(mp4_box(b"data", &data));
            let mut meta = vec![0; 4];
            meta.extend(mp4_box(b"hdlr", &[0; 25]));
            meta.extend(mp4_box(b"ilst", &mp4_box(b"----", &item)));
            moov.extend(mp4_box(b"udta", &mp4_box(b"meta", &meta)));
        }
        let mut file = mp4_box(b"ftyp", b"M4A \0\0\0\0M4A mp42isom");
        file.extend(mp4_box(b"mdat", &[0x21; 300]));
        file.extend(mp4_box(b"moov", &moov));
        file
    }

    /// An M4A's encoder priming and padding are read from the file, so its audio starts and
    /// ends where the recording did: ffmpeg's edit list, or Apple's `iTunSMPB` tag when
    /// there is no edit list. An empty edit is passed over; a file with neither is left
    /// whole.
    #[test]
    fn an_m4a_says_how_much_priming_and_padding_to_drop() {
        let trim = |bytes: Vec<u8>| mp4_trim(&mut std::io::Cursor::new(bytes));
        // ffmpeg at 16 kHz: 1,024 samples of priming, 11 s kept.
        assert_eq!(
            trim(mp4(16_000, 16_000, &[(176_000, 1_024)], None)),
            Some(Trim {
                skip: 1_024,
                keep: Some(176_000),
                timescale: 16_000
            })
        );
        // A movie timescale of 1,000: the duration is turned into the track's 44.1 kHz.
        let t = trim(mp4(1_000, 44_100, &[(0, -1), (3_000, 2_112)], None)).unwrap();
        assert_eq!(t.at_rate(44_100), (2_112, Some(132_300)));
        // Apple: no edit list, iTunSMPB with 2,112 of priming and 39,308,118 samples.
        let apple = " 00000000 00000840 0000006A 000000000257CB56 00000000 00000000";
        let t = trim(mp4(48_000, 48_000, &[], Some(apple))).unwrap();
        assert_eq!(t.at_rate(48_000), (2_112, Some(39_308_118)));
        // An edit that starts at zero says nothing; the tag then decides.
        let t = trim(mp4(48_000, 48_000, &[(1_000, 0)], Some(apple))).unwrap();
        assert_eq!(t.skip, 2_112);
        assert_eq!(trim(mp4(48_000, 48_000, &[], None)), None);
        assert_eq!(trim(b"not an mp4 file".to_vec()), None);
        assert!(is_mp4(&mp4(1, 1, &[], None)) && !is_mp4(b"RIFF....WAVE"));
    }

    /// A trim drops the first `skip` frames and keeps `keep`, whatever the packets' sizes.
    #[test]
    fn a_trim_cuts_the_decoded_audio_at_the_source_rate() {
        let mono = tone(16_000, 2.0);
        let whole = decode_reader(wav(16_000, 1, false, &mono).as_slice()).unwrap();
        let trimmed = decode(
            Box::new(std::io::Cursor::new(wav(16_000, 1, false, &mono))),
            Some("wav"),
            None,
            Trim {
                skip: 1_024,
                keep: Some(16_000),
                timescale: 0,
            },
        )
        .unwrap();
        assert_eq!(trimmed.samples.len(), 16_000);
        assert_eq!(trimmed.samples, whole.samples[1_024..17_024]);
    }

    /// Encoder output, a few KB each (52,577 B in all), made on macOS with ffmpeg 9.0.2 and
    /// `afconvert` from [`chirp`] at 48 kHz mono 16 bit, so the default tests decode the
    /// codecs O-39 and O-41 ship rather than only parsing their containers (removing
    /// symphonia's `aac`, `isomp4`, `vorbis`, `ogg` or `alac` feature fails them). They live with the other
    /// dictation fixtures, inside the budget `scripts/check-dictation-fixtures.sh` holds
    /// (DD-95):
    /// - `chirp-apple.m4a`: `afconvert -f m4af -d aac@48000 -b 64000`, AAC-LC with 2,112
    ///   samples of priming, an `iTunSMPB` tag and no edit list;
    /// - `chirp-ffmpeg.m4a`: `ffmpeg -bitexact -c:a aac -b:a 32k -map_metadata -1`, AAC-LC
    ///   with 1,024 samples of priming in an edit list;
    /// - `chirp-apple.aac`: `afconvert -f adts -d aac@48000 -b 64000`, MPEG-2 ADTS (0xFFF9);
    /// - `chirp-vorbis.ogg`: `ffmpeg -bitexact -c:a vorbis -strict -2 -ac 2 -map_metadata -1`,
    ///   stereo;
    /// - `chirp-alac.m4a`: `afconvert -f m4af -d alac`, Apple Lossless as Voice Memos writes
    ///   it with Lossless on, from a 16 bit WAV quantized as [`wav`] does (4,096 samples a
    ///   packet, an `iTunSMPB` tag, the `moov` box before the audio).
    const APPLE_M4A: &[u8] =
        include_bytes!("../../../biorouter-dictation/tests/fixtures/chirp-apple.m4a");
    const FFMPEG_M4A: &[u8] =
        include_bytes!("../../../biorouter-dictation/tests/fixtures/chirp-ffmpeg.m4a");
    const APPLE_ADTS: &[u8] =
        include_bytes!("../../../biorouter-dictation/tests/fixtures/chirp-apple.aac");
    const VORBIS_OGG: &[u8] =
        include_bytes!("../../../biorouter-dictation/tests/fixtures/chirp-vorbis.ogg");
    const ALAC_M4A: &[u8] =
        include_bytes!("../../../biorouter-dictation/tests/fixtures/chirp-alac.m4a");

    /// The fixtures' signal at `rate`: one second of a linear chirp from 300 Hz to 3,000 Hz
    /// at 0.3 of full scale, with 10 ms raised cosine fades.
    fn chirp(rate: u32) -> Vec<f32> {
        let fade = 0.010;
        (0..rate)
            .map(|i| {
                let t = f64::from(i) / f64::from(rate);
                let phase =
                    2.0 * std::f64::consts::PI * (300.0 * t + (3_000.0 - 300.0) * t * t / 2.0);
                let edge = t.min(1.0 - t);
                let gain = if edge < fade {
                    0.5 - 0.5 * (std::f64::consts::PI * edge / fade).cos()
                } else {
                    1.0
                };
                (0.3 * phase.sin() * gain) as f32
            })
            .collect()
    }

    /// The lag, in samples, at which `decoded` best matches the first half second of
    /// `reference` (positive when the decoded audio starts late; a chirp matches itself at
    /// one lag only), and how well it matches there, from -1 to 1 whatever the level.
    fn best_match(
        decoded: &[i16],
        reference: &[f32],
        lags: std::ops::RangeInclusive<i64>,
    ) -> (i64, f64) {
        let reference = &reference[..reference.len() / 2];
        let pairs = |lag: i64| {
            reference.iter().enumerate().filter_map(move |(i, &r)| {
                let at = usize::try_from(i as i64 + lag).ok()?;
                Some((f64::from(*decoded.get(at)?), f64::from(r)))
            })
        };
        let score = |lag: i64| -> f64 { pairs(lag).map(|(d, r)| d * r).sum() };
        let lag = lags.max_by(|a, b| score(*a).total_cmp(&score(*b))).unwrap();
        let (dd, rr) = pairs(lag).fold((0.0, 0.0), |(dd, rr), (d, r)| (dd + d * d, rr + r * r));
        (lag, score(lag) / (dd * rr).sqrt())
    }

    /// Real M4A (both encoders' priming), MPEG-2 ADTS and Ogg Vorbis files decode to 16 kHz
    /// mono at their length, from a file and from standard input alike. An M4A's priming is
    /// gone: its chirp starts where the recording's did, while the same file decoded with
    /// no trim starts late by exactly the priming, which is what symphonia 0.5 alone gives.
    /// A `.aac` says nothing of its priming, so it keeps Apple's 2,112 samples (44 ms).
    #[test]
    fn real_m4a_aac_and_ogg_vorbis_decode_in_time() {
        struct Case {
            name: &'static str,
            bytes: &'static [u8],
            channels: usize,
            /// Where the chirp starts, in 16 kHz samples.
            lag: i64,
            /// The length, in 16 kHz samples.
            length: std::ops::RangeInclusive<usize>,
            /// An M4A's lag decoded with no trim: its priming.
            untrimmed: Option<i64>,
        }
        let dir = tempfile::tempdir().unwrap();
        let reference = chirp(16_000);
        // 2,112 and 1,024 samples of priming at 48 kHz are 704 and 341.33 at 16 kHz. The
        // `.aac` is 49 frames of 1,024 samples at 48 kHz, priming and padding included.
        // Measured: lags 0, 0, 704 and 0, lengths 16,000, 16,000, 16,725 and 16,000, and a
        // likeness of 0.9998, 0.976 (ffmpeg's encoder at 32 kb/s), 0.9998 and 0.99999.
        let cases = [
            Case {
                name: "Voice Memo.m4a",
                bytes: APPLE_M4A,
                channels: 1,
                lag: 0,
                length: 15_998..=16_002,
                untrimmed: Some(704),
            },
            Case {
                name: "ffmpeg.m4a",
                bytes: FFMPEG_M4A,
                channels: 1,
                lag: 0,
                length: 15_998..=16_002,
                untrimmed: Some(341),
            },
            Case {
                name: "Voice Memo.aac",
                bytes: APPLE_ADTS,
                channels: 1,
                lag: 704,
                length: 16_723..=16_727,
                untrimmed: None,
            },
            Case {
                name: "vorbis.ogg",
                bytes: VORBIS_OGG,
                channels: 2,
                lag: 0,
                length: 15_998..=16_002,
                untrimmed: None,
            },
        ];
        for Case {
            name,
            bytes,
            channels,
            lag: expected_lag,
            length,
            untrimmed,
        } in cases
        {
            let path = write(dir.path(), name, bytes);
            let decoded = decode_path(&path).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(
                (decoded.source_rate, decoded.channels),
                (48_000, channels),
                "{name}"
            );
            assert!(
                length.contains(&decoded.samples.len()),
                "{name}: {} samples",
                decoded.samples.len()
            );
            let (found, likeness) = best_match(&decoded.samples, &reference, -200..=1_000);
            assert!(
                (found - expected_lag).abs() <= 2,
                "{name}: lag {found}, expected {expected_lag}"
            );
            // The audio itself, not only its timing: the chirp, as lossy codecs keep it.
            assert!(likeness > 0.95, "{name}: likeness {likeness}");
            let piped = decode_reader(bytes).unwrap_or_else(|e| panic!("{name} on stdin: {e:?}"));
            assert_eq!(piped, decoded, "{name} on stdin");
            if let Some(priming) = untrimmed {
                let whole = decode(
                    Box::new(std::io::Cursor::new(bytes.to_vec())),
                    Some("m4a"),
                    None,
                    Trim::NONE,
                )
                .unwrap();
                let (found, _) = best_match(&whole.samples, &reference, -200..=1_000);
                assert!(
                    (found - priming).abs() <= 2,
                    "{name} untrimmed: lag {found}"
                );
            }
        }
    }

    /// An Apple Lossless M4A (a voice memo recorded with Lossless on, O-41) decodes, from a
    /// file and from standard input, to exactly what the same 16 bit samples give as a WAV:
    /// the codec is lossless, so only the resampling to 16 kHz touches them, the same for
    /// both.
    #[test]
    fn an_apple_lossless_m4a_decodes_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "New Recording.m4a", ALAC_M4A);
        let decoded = decode_path(&path).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!((decoded.source_rate, decoded.channels), (48_000, 1));
        assert_eq!(decoded.samples.len(), 16_000);
        let same = write(
            dir.path(),
            "same.wav",
            &wav(48_000, 1, false, &chirp(48_000)),
        );
        assert_eq!(decoded, decode_path(&same).unwrap(), "lossless");
        assert_eq!(decode_reader(ALAC_M4A).unwrap(), decoded, "on stdin");
    }

    /// A damaged Apple Lossless M4A fails cleanly: cut short at any point, or with any one
    /// byte changed, it decodes what it can or is refused, and never panics, though
    /// symphonia's MP4 reader panics on some of these bytes (a damaged `stsc` box) and
    /// [`decode`] catches it. A decoder configuration with a bit depth of 0 is refused as a
    /// recording Biorouter cannot read.
    #[test]
    fn a_damaged_apple_lossless_m4a_fails_cleanly() {
        let whole = decode_reader(ALAC_M4A).unwrap().samples.len();
        let find = |name: &[u8]| {
            ALAC_M4A
                .windows(4)
                .position(|w| w == name)
                .expect("a box of the fixture")
        };
        // The boxes that describe the audio come first (`moov`), the audio after (`mdat`):
        // the parser is read densely, the codec's packets sparsely, so the debug build's run
        // stays at a few seconds. Of every one byte change to `moov`, only the entry counts
        // of `stts` and `stsc` made symphonia panic [M, 2026-11-13: offsets 486 to 488 and
        // 518 to 521 of 4,092]; a cut never did.
        let audio = find(b"mdat");
        for at in (0..audio)
            .step_by(5)
            .chain((audio..ALAC_M4A.len()).step_by(401))
        {
            if let Ok(decoded) = decode_reader(&ALAC_M4A[..at]) {
                assert!(decoded.samples.len() <= whole, "cut at {at}");
            }
        }
        for at in (0..audio)
            .step_by(31)
            .chain((audio..ALAC_M4A.len()).step_by(1_601))
        {
            let mut bytes = ALAC_M4A.to_vec();
            bytes[at] ^= 0xA5;
            // Any answer will do, an error or audio; returning at all is the test.
            let _ = decode_reader(bytes.as_slice());
        }
        // The panic itself, caught: a damaged `stsc` entry count.
        let mut bytes = ALAC_M4A.to_vec();
        bytes[find(b"stsc") + 8] ^= 0xA5;
        assert_eq!(
            decode_reader(bytes.as_slice()).unwrap_err(),
            DecodeError::UnsupportedFormat { kind: None }
        );
        // The `alac` box inside the sample entry: size, type, version and flags, then the
        // decoder's configuration, whose sixth byte is the bit depth.
        let entry = ALAC_M4A
            .windows(4)
            .position(|w| w == b"alac")
            .expect("the sample entry");
        let config = entry
            + 4
            + ALAC_M4A[entry + 4..]
                .windows(4)
                .position(|w| w == b"alac")
                .expect("the decoder's configuration");
        let depth = config + 4 + 4 + 5;
        assert_eq!(ALAC_M4A[depth], 16, "the fixture's bit depth");
        let mut bytes = ALAC_M4A.to_vec();
        bytes[depth] = 0;
        assert_eq!(
            decode_reader(bytes.as_slice()).unwrap_err(),
            DecodeError::UnsupportedFormat { kind: None }
        );
    }

    /// The MPEG-2 ADTS header walk adds up its frames' audio and stops at 2 hours, so the
    /// list it keeps stays bounded whatever the file holds: frames of 7 bytes with no
    /// payload, at 8 kHz, are 1,024 samples each, and 56,250 of them are 2 hours exactly.
    #[test]
    fn an_mpeg2_adts_stream_over_2_hours_is_refused_while_its_headers_are_walked() {
        // Profile LC, 8 kHz (index 11), one channel, a 7 byte frame, one raw data block.
        let frame = [0xFF, 0xF9, 0x6C, 0x40, 0x00, 0xFF, 0xFC];
        assert_eq!(adts_frame(&frame), Some((7, 1_024, 8_000)));
        let two_hours = frame.repeat(56_250);
        let headers = mpeg2_adts_headers(&mut std::io::Cursor::new(&two_hours)).unwrap();
        assert_eq!(headers.len(), 56_250);
        let mut longer = two_hours;
        longer.extend_from_slice(&frame);
        assert_eq!(
            mpeg2_adts_headers(&mut std::io::Cursor::new(&longer)).unwrap_err(),
            DecodeError::TooLong
        );
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "long.aac", &longer);
        assert_eq!(decode_path(&path).unwrap_err(), DecodeError::TooLong);
    }

    /// A file's trim values and box sizes can be anything: none of them overflows, and a
    /// 64 bit box size that would carry the box walk past `u64::MAX` (and wrap it back to a
    /// box already walked, for ever) ends the walk.
    #[test]
    fn hostile_trims_and_box_sizes_neither_overflow_nor_loop() {
        let huge = Trim {
            skip: u64::MAX,
            keep: Some(u64::MAX),
            timescale: 1,
        };
        assert_eq!(huge.at_rate(96_000), (u64::MAX, Some(u64::MAX)));
        // `skip + keep` past `u64::MAX` keeps everything after `skip`.
        let decoded = decode(
            Box::new(std::io::Cursor::new(wav(
                16_000,
                1,
                false,
                &tone(16_000, 1.0),
            ))),
            Some("wav"),
            None,
            Trim {
                skip: 1,
                keep: Some(u64::MAX),
                timescale: 0,
            },
        )
        .unwrap();
        assert_eq!(decoded.samples.len(), 15_999);
        // A version 1 edit list of the largest duration and media time.
        let mut elst = vec![1, 0, 0, 0, 0, 0, 0, 1];
        elst.extend_from_slice(&u64::MAX.to_be_bytes());
        elst.extend_from_slice(&i64::MAX.to_be_bytes());
        elst.extend_from_slice(&[0, 1, 0, 0]);
        let trim = mp4_trim(&mut std::io::Cursor::new(mp4_with(
            1,
            48_000,
            Some(elst),
            None,
        )));
        assert_eq!(
            trim,
            Some(Trim {
                skip: i64::MAX as u64,
                keep: Some(u64::MAX),
                timescale: 48_000
            })
        );
        // `ftyp`, then a box whose 64 bit size reaches one byte past `u64::MAX`.
        let mut file = mp4_box(b"ftyp", b"M4A \0\0\0\0");
        file.extend_from_slice(&1u32.to_be_bytes());
        file.extend_from_slice(b"free");
        file.extend_from_slice(&(u64::MAX - 15).to_be_bytes());
        let len = file.len() as u64;
        assert_eq!(
            find_box(&mut std::io::Cursor::new(&file), 0, len, b"moov"),
            None
        );
        assert_eq!(mp4_trim(&mut std::io::Cursor::new(file)), None);
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
