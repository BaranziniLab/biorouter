//! What `biorouter transcribe` says (PR-12), word for word as the message catalog in
//! `docs/dictation/surfaces.md` has it (MSG-TRANSCRIBE-NOTHING to MSG-TRANSCRIBE-FAILED,
//! with MSG-TOO-SHORT and the first sentence of MSG-ENGINE-FAILED-CLI), each pinned by
//! `the_transcribe_sentences_are_the_catalogs` below. `dictate` adds its own from PR-25.

use std::path::Path;

/// MSG-TRANSCRIBE-NOTHING: no speech, an echo only, or exact zeros.
pub const MSG_NO_SPEECH: &str =
    "No speech was heard in that recording, so nothing was transcribed.";
/// MSG-TOO-SHORT.
pub const MSG_TOO_SHORT: &str =
    "That recording was under half a second, so nothing was transcribed.";
/// The first sentence of MSG-ENGINE-FAILED-CLI.
pub const MSG_ENGINE_FAILED: &str = "Transcription failed.";
/// MSG-TRANSCRIBE-CANCELLED: Ctrl-C.
pub const MSG_CANCELLED: &str = "Transcription was cancelled.";

/// MSG-TRANSCRIBE-FAILED: MSG-ENGINE-FAILED-CLI's first sentence, then the engine's
/// diagnostic (one of MSG-DIAG-EXIT to MSG-DIAG-CANCELLED), which names no transcript and
/// no audio.
pub fn msg_engine_failed(detail: &str) -> String {
    format!("{MSG_ENGINE_FAILED} {detail}")
}

/// MSG-TRANSCRIBE-TOO-LONG: over 2 hours, or over 200 MB on standard input (DD-78). The
/// `ffmpeg` line converts while it splits, so every part, an hour of 16 kHz mono at
/// about 115 MB, passes both limits.
pub fn msg_too_long(input: &str) -> String {
    let file = if input == "-" {
        "<file>".to_string()
    } else {
        shell_word(input)
    };
    format!(
        "That recording is longer than 2 hours, or larger than 200 MB on standard input. Split it into 16 kHz mono parts first, for example: ffmpeg -i {file} -ar 16000 -ac 1 -f segment -segment_time 3600 part%02d.wav"
    )
}

/// MSG-TRANSCRIBE-UNSUPPORTED: a recording kind Biorouter does not decode (Opus, WebM,
/// Apple Lossless, or anything else symphonia does not read), with the `ffmpeg` line that
/// converts it (DD-78; since O-39 WAV, AIFF, FLAC, MP3, M4A, AAC and Ogg Vorbis are read).
/// `kind` names it when it can be named; `input` is the file as the person named it, `-`
/// for standard input. The example writes the file's name with `.wav`, or with `-16k.wav`
/// when the file is itself a `.wav` (an ADPCM recording, a WebM renamed), so it never
/// writes over the file it reads: ffmpeg refuses that line.
pub fn msg_unsupported_format(kind: Option<&str>, input: &str) -> String {
    let convert = if input == "-" {
        "ffmpeg -i <file> -ar 16000 -ac 1 -f wav - | biorouter transcribe -".to_string()
    } else {
        let path = Path::new(input);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("recording");
        let is_wav = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("wav"));
        let name = if is_wav {
            format!("{stem}-16k")
        } else {
            stem.to_string()
        };
        format!(
            "ffmpeg -i {} -ar 16000 -ac 1 {}.wav",
            shell_word(input),
            shell_word(&name)
        )
    };
    let what = match kind {
        Some(kind) => format!("{kind} recordings"),
        None => "this recording".to_string(),
    };
    format!("Biorouter cannot read {what}. Convert it to WAV first, for example: {convert}")
}

/// MSG-TRANSCRIBE-NOT-FOUND.
pub fn msg_not_found(input: &str) -> String {
    format!("There is no file at {input}.")
}

/// MSG-TRANSCRIBE-UNREADABLE.
pub fn msg_unreadable(input: &str) -> String {
    if input == "-" {
        "Standard input could not be read.".to_string()
    } else {
        format!("{input} could not be read.")
    }
}

/// MSG-TRANSCRIBE-UNWRITABLE: `--output`, or standard output when `path` is `None`.
pub fn msg_unwritable(path: Option<&Path>) -> String {
    match path {
        Some(path) => format!("The transcript could not be written to {}.", path.display()),
        None => "The transcript could not be written to standard output.".to_string(),
    }
}

/// `word` quoted for a POSIX shell when it holds anything but plain characters.
fn shell_word(word: &str) -> String {
    let plain = word
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "._-/+=:@,".contains(c));
    if plain && !word.is_empty() {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `transcribe`'s sentences, word for word as the message catalog has them.
    #[test]
    fn the_transcribe_sentences_are_the_catalogs() {
        let cases: Vec<(String, &str)> = vec![
            (MSG_NO_SPEECH.into(), "No speech was heard in that recording, so nothing was transcribed."),
            (MSG_TOO_SHORT.into(), "That recording was under half a second, so nothing was transcribed."),
            (MSG_CANCELLED.into(), "Transcription was cancelled."),
            (MSG_ENGINE_FAILED.into(), "Transcription failed."),
            (msg_engine_failed("The dictation worker was stopped."), "Transcription failed. The dictation worker was stopped."),
            (msg_too_long("memo.wav"), "That recording is longer than 2 hours, or larger than 200 MB on standard input. Split it into 16 kHz mono parts first, for example: ffmpeg -i memo.wav -ar 16000 -ac 1 -f segment -segment_time 3600 part%02d.wav"),
            (msg_too_long("-"), "That recording is longer than 2 hours, or larger than 200 MB on standard input. Split it into 16 kHz mono parts first, for example: ffmpeg -i <file> -ar 16000 -ac 1 -f segment -segment_time 3600 part%02d.wav"),
            (msg_unsupported_format(Some("Opus"), "memo.opus"), "Biorouter cannot read Opus recordings. Convert it to WAV first, for example: ffmpeg -i memo.opus -ar 16000 -ac 1 memo.wav"),
            (msg_unsupported_format(Some("Apple Lossless"), "New Recording.m4a"), "Biorouter cannot read Apple Lossless recordings. Convert it to WAV first, for example: ffmpeg -i 'New Recording.m4a' -ar 16000 -ac 1 'New Recording'.wav"),
            (msg_unsupported_format(None, "notes.txt"), "Biorouter cannot read this recording. Convert it to WAV first, for example: ffmpeg -i notes.txt -ar 16000 -ac 1 notes.wav"),
            (msg_unsupported_format(None, "recorder.wav"), "Biorouter cannot read this recording. Convert it to WAV first, for example: ffmpeg -i recorder.wav -ar 16000 -ac 1 recorder-16k.wav"),
            (msg_unsupported_format(None, "-"), "Biorouter cannot read this recording. Convert it to WAV first, for example: ffmpeg -i <file> -ar 16000 -ac 1 -f wav - | biorouter transcribe -"),
            (msg_not_found("memo.wav"), "There is no file at memo.wav."),
            (msg_unreadable("memo.wav"), "memo.wav could not be read."),
            (msg_unreadable("-"), "Standard input could not be read."),
            (msg_unwritable(Some(Path::new("/notes/memo.txt"))), "The transcript could not be written to /notes/memo.txt."),
            (msg_unwritable(None), "The transcript could not be written to standard output."),
        ];
        for (sentence, catalog) in cases {
            assert_eq!(sentence, catalog);
        }
    }

    #[test]
    fn the_ffmpeg_lines_quote_what_a_shell_would_split() {
        assert!(msg_unsupported_format(Some("WebM"), "my talk.webm")
            .ends_with("ffmpeg -i 'my talk.webm' -ar 16000 -ac 1 'my talk'.wav"));
        assert!(msg_too_long("it's.wav").contains("ffmpeg -i 'it'\\''s.wav' -ar 16000"));
        assert!(msg_unsupported_format(Some("WebM"), "-").contains("| biorouter transcribe -"));
    }
}
