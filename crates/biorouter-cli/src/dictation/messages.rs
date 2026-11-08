//! What `biorouter transcribe` (PR-12) and `biorouter dictate` (PR-25) say, word for word
//! as the message catalog in `docs/dictation/surfaces.md` has it: MSG-TRANSCRIBE-NOTHING to
//! MSG-TRANSCRIBE-FAILED, with MSG-TOO-SHORT and the first sentence of
//! MSG-ENGINE-FAILED-CLI, pinned by `the_transcribe_sentences_are_the_catalogs`; and the
//! terminal's live dictation sentences (MSG-AGENT-SHELL to MSG-CLIPBOARD-WAIT), pinned by
//! `the_dictate_sentences_are_the_catalogs`, which reads the catalog's rows themselves.

use std::path::Path;

/// MSG-AGENT-SHELL: step 3 of the detection order (exit 2).
pub const MSG_AGENT_SHELL: &str = "Dictation records your microphone, so only you can start it, from your own terminal. Biorouter does not start it from an agent's shell.";
/// MSG-NO-TTY: step 4 (exit 2).
pub const MSG_NO_TTY: &str = "Live dictation needs an interactive terminal. To transcribe a recording instead, run: biorouter transcribe <file>";
/// MSG-SSH-MAC: step 5, followed by the routes.
pub const MSG_SSH_MAC: &str = "macOS only lets apps in the logged in desktop session use the microphone, and this is an SSH session.";
/// MSG-CONTAINER.
pub const MSG_CONTAINER: &str = "This container has no microphone. Transcribe a recording with `biorouter transcribe <file>`, or run `biorouter serve` and dictate from your browser.";
/// MSG-LINUX-NOAUDIO.
pub const MSG_LINUX_NOAUDIO: &str =
    "No audio system is running here (no PulseAudio, PipeWire or ALSA capture device).";
/// MSG-LINUX-ROUTES, always after MSG-LINUX-NOAUDIO.
pub const MSG_LINUX_ROUTES: &str = "To dictate with this computer's model and dictionary, run `biorouter serve` here and open it from your own computer through an SSH tunnel or an HTTPS address, or transcribe a recording with `biorouter transcribe <file>`.";
/// MSG-WSL2-NOPULSE.
pub const MSG_WSL2_NOPULSE: &str = "WSL delivers audio through WSLg's PulseAudio server. Install the PulseAudio client library (`sudo apt install libpulse0`), then try again.";
/// MSG-WSL2-NOWSLG.
pub const MSG_WSL2_NOWSLG: &str = "This WSL has no audio, because WSLg is not available or is turned off. Use Biorouter on Windows, run `biorouter serve` here and open the address it prints in a Windows browser, or transcribe a recording with `biorouter transcribe <file>`.";
/// MSG-WSL1.
pub const MSG_WSL1: &str = "WSL1 has no audio. Use Biorouter on Windows, or WSL2 with WSLg.";
/// MSG-WIN-ZERO: step 10 on Windows (exit 3).
pub const MSG_WIN_ZERO: &str = "Windows gave Biorouter only silence. Turn on “Let desktop apps access your microphone” in Settings > Privacy & security > Microphone.";
/// MSG-SILENT: step 10 on Linux (exit 3).
pub const MSG_SILENT: &str = "The microphone sent only silence. Check that it is not muted.";
/// MSG-NO-AUDIO-YET: below the noise floor for 3 s; the recording goes on.
pub const MSG_NO_AUDIO_YET: &str = "No audio detected. Check that your microphone is not muted.";
/// MSG-CAP-WARN, at 570 s.
pub const MSG_CAP_WARN: &str =
    "Dictation stops in 30 seconds. What you said so far will be inserted.";
/// MSG-CAP-STOPPED, at 600 s.
pub const MSG_CAP_STOPPED: &str =
    "Dictation stopped at the 10 minute limit and inserted what you said.";
/// MSG-PAUSED: three failures within 10 s.
pub const MSG_PAUSED: &str = "Dictation failed three times in a row, so it is paused for this session. Type /dictate status to see why.";
pub const MSG_KEY_CLASH: &str = "Ctrl+<key> dictation is off because BIOROUTER_CLI_NEWLINE_KEY uses the same key. Set BIOROUTER_CLI_DICTATION_KEY to another letter.";
pub fn msg_key_clash(key: char) -> String {
    MSG_KEY_CLASH.replace("<key>", &key.to_ascii_uppercase().to_string())
}
/// MSG-NO-DEVICE: step 7 elsewhere, and step 9 when the device is gone.
pub const MSG_NO_DEVICE: &str =
    "No microphone found. Connect one or make it the system default, then try again.";
/// MSG-MIC-BUSY: step 9 when the device is there but will not open.
pub const MSG_MIC_BUSY: &str = "The microphone is in use by another app, or the system blocked it.";
/// MSG-ENGINE-FAILED-CLI, as the terminal front ends print it; `dictate` prints its first
/// sentence, [`MSG_ENGINE_FAILED`].
pub const MSG_ENGINE_FAILED_CLI: &str =
    "Transcription failed. Type /dictate retry within 5 minutes to try the same recording again.";
/// MSG-DIAG-CANCELLED: `dictate` cancelled with Escape or Ctrl-C, or a review discarded
/// (exit 4).
pub const MSG_DICTATION_CANCELLED: &str = "Dictation was cancelled.";
/// MSG-PRIVATE-TERMS, without the desktop's button labels; the terminal adds `[y/N]`.
pub const MSG_PRIVATE_TERMS: &str = "Your private dictionary terms may appear in this text, and Biorouter cannot keep them away from public models or from other institutions' models once you paste or post it. Include them?";
/// MSG-PRIVATE-TERMS-REFUSED (exit 2).
pub const MSG_PRIVATE_TERMS_REFUSED: &str = "Private dictionary terms need you at your own terminal and a private model configured on this computer, so `--private-terms` was refused. Run the command without it to dictate with public terms.";
/// MSG-CLIPBOARD-NOTHING (exit 3).
pub const MSG_CLIPBOARD_NOTHING: &str = "Nothing was heard, so the clipboard was not changed.";
/// MSG-CLIPBOARD-ECHO (exit 3).
pub const MSG_CLIPBOARD_ECHO: &str = "Only dictionary words came back, which usually means the recording was silent. Nothing was copied.";
/// MSG-NO-CLIPBOARD (exit 5), with the text printed on stdout.
pub const MSG_NO_CLIPBOARD: &str =
    "No clipboard is reachable here, so the text is printed below. Copy it from the terminal.";
/// MSG-CLIPBOARD-WAIT: Linux, when the copy is still being served after 300 ms.
pub const MSG_CLIPBOARD_WAIT: &str = "Copied. Waiting up to 60 s for you to paste…";
/// MSG-NO-HISTORY-WSL (exit 2): `--copy --no-history` under WSL, whose `powershell.exe`
/// route cannot mark a copy to be left out of Windows clipboard history. Not yet a row of
/// the catalog in `docs/dictation/surfaces.md`.
pub const MSG_NO_HISTORY_WSL: &str = "Windows cannot be told to keep a copy made from WSL out of its clipboard history, so `--no-history` was refused. Run the command without `--copy` to print the text instead, or without `--no-history` to copy it anyway.";

/// MSG-SSH-ROUTES, with `<host>` filled in.
pub fn msg_ssh_routes(host: &str) -> String {
    format!(
        "This computer has no microphone you can reach over SSH. To dictate with this computer's model and dictionary, run `biorouter serve` here and keep it running, then on your laptop run `ssh -N -L 8765:127.0.0.1:8765 {host}` and open the address `serve` prints, including its ?t= part. Or dictate on your own computer with `biorouter dictate --copy` and paste, or copy a recording here and run `biorouter transcribe <file>`."
    )
}

/// MSG-SSH-SERVICE: a `biorouter serve` already runs here on `port`; `<token>` is left as
/// written, because only whoever runs the service has it.
pub fn msg_ssh_service(port: &str, host: &str) -> String {
    format!(
        "A Biorouter service already runs on this computer, on port {port}. To dictate with it, on your laptop run `ssh -N -L {port}:127.0.0.1:{port} {host}` and open http://127.0.0.1:{port}/?t=<token>, with the token from whoever runs the service. Or dictate on your own computer with `biorouter dictate --copy` and paste, or copy a recording here and run `biorouter transcribe <file>`."
    )
}

/// MSG-SYNC-HINT, after the routes over SSH.
pub fn msg_sync_hint(host: &str) -> String {
    format!(
        "To use your laptop's dictionary terms here, run on your laptop: `biorouter dictionary export - | ssh {host} biorouter dictionary import -`"
    )
}

/// MSG-TCC-ZERO: step 10 on macOS, naming the terminal app and its bundle id.
pub fn msg_tcc_zero(terminal_app: &str, bundle_id: &str) -> String {
    format!(
        "macOS gave Biorouter only silence, which usually means {terminal_app} is not allowed to use the microphone. Turn it on in System Settings > Privacy & Security > Microphone, then quit and reopen it. If it is not listed: `tccutil reset Microphone {bundle_id}`. If {terminal_app} never asks, it cannot use the microphone on macOS: run Biorouter in Terminal, iTerm2, Ghostty or VS Code instead, or use Dictate to clipboard in the Biorouter app."
    )
}

/// MSG-NO-MODEL-CLI: step 8.
pub fn msg_no_model_cli(host: &str, user: &str, model: &str, size: &str) -> String {
    format!(
        "Dictation needs a speech model on this computer ({host}, account {user}). Install {model} ({size}, from BaranziniLab's GitHub release)? Run `biorouter dictation install`, or type `/dictate install`."
    )
}

/// MSG-CLIPBOARD-COPIED.
pub fn msg_clipboard_copied(words: impl std::fmt::Display, key: &str) -> String {
    format!("Copied {words} words. Paste with {key}.")
}

/// The recording line, until the first frame with a level above zero.
pub const LINE_STARTING: &str = "◌ Starting microphone…";
/// The review's keys, under the transcript (DD-76).
pub const LINE_REVIEW: &str = "Enter use · e edit · Esc discard";

/// `m:ss` of a duration in milliseconds.
pub fn clock(ms: u64) -> String {
    let seconds = ms / 1_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// One cell of the level meter for a period's peak.
pub fn meter_cell(peak: f32) -> char {
    const CELLS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    // Speech peaks sit between about 0.05 and 0.5; a square root spreads them over the
    // eight cells.
    let scaled = (peak.clamp(0.0, 1.0).sqrt() * 8.0) as usize;
    CELLS[scaled.min(CELLS.len() - 1)]
}

/// The recording line: `● REC 0:07 ▂▅▇▅▂  Enter stop · Esc cancel`.
pub fn line_recording(ms: u64, peaks: &[f32]) -> String {
    let meter: String = peaks.iter().map(|p| meter_cell(*p)).collect();
    format!("● REC {} {meter}  Enter stop · Esc cancel", clock(ms))
}

/// The line while the last of the speech is decoded.
pub fn line_transcribing(ms: u64) -> String {
    format!("◌ Transcribing {}…", clock(ms))
}

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

    /// The text column of `id`'s row in the catalog, unescaped: `\<x>` reads `<x>` and
    /// `\|` reads `|`, as the rendered table shows them.
    fn catalog_row(catalog: &str, id: &str) -> String {
        let head = format!("| {id} |");
        let row = catalog
            .lines()
            .find(|l| l.starts_with(&head))
            .unwrap_or_else(|| panic!("{id} has no row in docs/dictation/surfaces.md"));
        row.trim_start_matches(&head)
            .split(" | ")
            .next()
            .unwrap()
            .trim()
            .replace("\\<", "<")
            .replace("\\|", "|")
    }

    /// `dictate`'s sentences, word for word as the catalog's rows have them, read from the
    /// catalog itself, so neither side can move alone. A sentence with places to fill is
    /// built with the catalog's own placeholders.
    #[test]
    fn the_dictate_sentences_are_the_catalogs() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/dictation/surfaces.md");
        let catalog = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            .replace("\r\n", "\n");
        let cases: Vec<(&str, String)> = vec![
            ("MSG-AGENT-SHELL", MSG_AGENT_SHELL.into()),
            ("MSG-NO-TTY", MSG_NO_TTY.into()),
            ("MSG-SSH-MAC", MSG_SSH_MAC.into()),
            ("MSG-SSH-ROUTES", msg_ssh_routes("<host>")),
            ("MSG-SSH-SERVICE", msg_ssh_service("<port>", "<host>")),
            ("MSG-SYNC-HINT", msg_sync_hint("<host>")),
            ("MSG-CONTAINER", MSG_CONTAINER.into()),
            ("MSG-LINUX-NOAUDIO", MSG_LINUX_NOAUDIO.into()),
            ("MSG-LINUX-ROUTES", MSG_LINUX_ROUTES.into()),
            ("MSG-WSL2-NOPULSE", MSG_WSL2_NOPULSE.into()),
            ("MSG-WSL2-NOWSLG", MSG_WSL2_NOWSLG.into()),
            ("MSG-WSL1", MSG_WSL1.into()),
            (
                "MSG-TCC-ZERO",
                msg_tcc_zero("<terminal app>", "<bundle id>"),
            ),
            ("MSG-WIN-ZERO", MSG_WIN_ZERO.into()),
            ("MSG-SILENT", MSG_SILENT.into()),
            ("MSG-NO-AUDIO-YET", MSG_NO_AUDIO_YET.into()),
            ("MSG-CAP-WARN", MSG_CAP_WARN.into()),
            ("MSG-CAP-STOPPED", MSG_CAP_STOPPED.into()),
            ("MSG-PAUSED", MSG_PAUSED.into()),
            ("MSG-NO-DEVICE", MSG_NO_DEVICE.into()),
            ("MSG-MIC-BUSY", MSG_MIC_BUSY.into()),
            ("MSG-ENGINE-FAILED-CLI", MSG_ENGINE_FAILED_CLI.into()),
            ("MSG-DIAG-CANCELLED", MSG_DICTATION_CANCELLED.into()),
            (
                "MSG-NO-MODEL-CLI",
                msg_no_model_cli("<host>", "<user>", "<model>", "<size>"),
            ),
            (
                "MSG-PRIVATE-TERMS-REFUSED",
                MSG_PRIVATE_TERMS_REFUSED.into(),
            ),
            ("MSG-CLIPBOARD-COPIED", msg_clipboard_copied("<n>", "<key>")),
            ("MSG-CLIPBOARD-NOTHING", MSG_CLIPBOARD_NOTHING.into()),
            ("MSG-CLIPBOARD-ECHO", MSG_CLIPBOARD_ECHO.into()),
            ("MSG-NO-CLIPBOARD", MSG_NO_CLIPBOARD.into()),
            ("MSG-CLIPBOARD-WAIT", MSG_CLIPBOARD_WAIT.into()),
            ("MSG-TOO-SHORT", MSG_TOO_SHORT.into()),
            ("MSG-TRANSCRIBE-NOTHING", MSG_NO_SPEECH.into()),
        ];
        for (id, sentence) in cases {
            assert_eq!(sentence, catalog_row(&catalog, id), "{id}");
        }
        // MSG-PRIVATE-TERMS carries the desktop's buttons after its sentence.
        assert!(catalog_row(&catalog, "MSG-PRIVATE-TERMS")
            .starts_with(&format!("{MSG_PRIVATE_TERMS} (buttons")));
        // The engine's own first sentence is MSG-ENGINE-FAILED-CLI's.
        assert!(MSG_ENGINE_FAILED_CLI.starts_with(MSG_ENGINE_FAILED));
    }

    #[test]
    fn the_copied_sentence_counts_words_and_names_the_key() {
        assert_eq!(
            msg_clipboard_copied(12, "⌘V"),
            "Copied 12 words. Paste with ⌘V."
        );
    }

    /// The recording lines `dictate` draws on the terminal (the TUI's own, PR-26, follow
    /// the same shapes).
    #[test]
    fn the_recording_lines_have_their_shapes() {
        assert_eq!(LINE_STARTING, "◌ Starting microphone…");
        assert_eq!(clock(7_400), "0:07");
        assert_eq!(clock(61_000), "1:01");
        assert_eq!(
            line_recording(7_000, &[0.01, 0.1, 0.3, 0.1, 0.01]),
            "● REC 0:07 ▁▃▅▃▁  Enter stop · Esc cancel"
        );
        assert_eq!(line_transcribing(7_000), "◌ Transcribing 0:07…");
        assert_eq!(meter_cell(0.0), '▁');
        assert_eq!(meter_cell(1.0), '█');
        assert_eq!(meter_cell(-3.0), '▁');
    }

    #[test]
    fn the_ffmpeg_lines_quote_what_a_shell_would_split() {
        assert!(msg_unsupported_format(Some("WebM"), "my talk.webm")
            .ends_with("ffmpeg -i 'my talk.webm' -ar 16000 -ac 1 'my talk'.wav"));
        assert!(msg_too_long("it's.wav").contains("ffmpeg -i 'it'\\''s.wav' -ar 16000"));
        assert!(msg_unsupported_format(Some("WebM"), "-").contains("| biorouter transcribe -"));
    }
}
