//! DD-93: a dictation log line carries only fields from one allowlist, and its message is
//! a fixed string.
//!
//! Logs never hold audio, transcripts, prompt strings, dictionary entries or `context`.
//! `dictation_routes.rs` checks that end to end for the lines a stream writes today; this
//! test holds the source to the list, so a line added tomorrow cannot log `text` or format
//! a transcript into its message without failing here first.
//!
//! It reads every `.rs` file under `crates/*/src` whose path names dictation, the
//! dictionary or `transcribe`, finds each `tracing` level macro (`info!`, `debug!`, ...,
//! with or without the `tracing::` path), and checks each of its arguments:
//!
//! - the last is a string literal with no `{`, so no value is formatted into the message;
//! - every other is a field named in [`ALLOWED_FIELDS`] (`name = value`, or the `%name`,
//!   `?name` and bare `name` shorthands) whose value names none of [`CONTENT_WORDS`];
//! - a `stream` field is a stream's log id (the digest `log_id_of` makes), never the id
//!   that opens the stream (DD-29).
//!
//! `event!`, `span!` and the `_span` macros take a level or a name first, so this reader
//! refuses them rather than guess: write the line with a level macro. Macro calls written
//! with `(`, `[` or `{`, and calls nested in another call's arguments, are all read.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::path::{Path, PathBuf};

/// The fields a dictation log line may carry (DD-93). None is audio, text, a prompt, a
/// dictionary entry or `context`.
const ALLOWED_FIELDS: &[(&str, &str)] = &[
    (
        "stream",
        "a stream's log id: 8 hex digits of its id's SHA-256",
    ),
    ("target_kind", "chat, new_chat, crew or external"),
    ("bias_scope", "the scope of the terms recognition used"),
    ("model", "a speech model id"),
    ("backend", "the compute backend that loaded a model"),
    ("why", "a warm up's reason, a fixed word"),
    ("sample_rate", "the capture's sample rate"),
    ("segments", "the decoder's segment count"),
    ("audio_ms", "the audio's length"),
    ("stop_to_text_ms", "stop to transcript"),
    ("wait_ms", "time queued for the worker"),
    ("tail_ms", "the last segment's decode"),
    ("target_ms", "the target decision"),
    ("store_us", "reading the dictionary store"),
    ("request_us", "the request's part of the target decision"),
    ("dictionary_us", "preparing the dictionary"),
    ("dictionary_ms", "preparing the dictionary"),
    ("elapsed_ms", "a model install's time"),
    ("ready_ms", "a warm up's load"),
    ("decode_ms", "a warm up's decode"),
];

/// Files whose names match but that hold no dictation data, each with the reason.
const NOT_DICTATION_DATA: &[(&str, &str)] = &[
    (
        "crates/biorouter/src/managed/dictation.rs",
        "logs why a managed policy file could not be read, and its path",
    ),
    (
        "crates/biorouter/src/security/dictionary_store.rs",
        "the store inspector's counters: a refused tool's name and request id, never a row",
    ),
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn is_dictation_source(relative: &str) -> bool {
    let lower = relative.to_ascii_lowercase();
    relative.ends_with(".rs")
        && lower.contains("/src/")
        && ["dictat", "dictionar", "transcribe"]
            .iter()
            .any(|word| lower.contains(word))
}

fn dictation_sources() -> Vec<(String, String)> {
    let root = repo_root();
    let mut found = Vec::new();
    let mut stack = vec![root.join("crates")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("a readable directory") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "target") {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if is_dictation_source(&relative)
                && !NOT_DICTATION_DATA.iter().any(|(file, _)| *file == relative)
            {
                let source = std::fs::read_to_string(&path).expect("a readable source file");
                found.push((relative, source));
            }
        }
    }
    found.sort();
    found
}

/// One macro call: the macro's name, its line, and its top level arguments, comments
/// left out.
struct Call {
    name: String,
    line: usize,
    args: Vec<String>,
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The index just past the string, raw string, byte string or char literal that starts
/// at `i`, or `None` when `i` starts none of them (a lifetime's `'` included).
fn skip_literal(bytes: &[u8], i: usize) -> Option<usize> {
    let starts_word = |at: usize| at == 0 || !is_ident(bytes[at - 1]);
    match bytes[i] {
        b'"' => {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'"' {
                j += if bytes[j] == b'\\' { 2 } else { 1 };
            }
            Some(j + 1)
        }
        // `r"…"`, `r#"…"#` and their byte forms `br"…"`, `br#"…"#`.
        b'r' if matches!(bytes.get(i + 1), Some(b'"' | b'#'))
            && (starts_word(i) || (bytes[i - 1] == b'b' && starts_word(i - 1))) =>
        {
            let mut j = i + 1;
            let mut hashes = 0;
            while bytes.get(j) == Some(&b'#') {
                hashes += 1;
                j += 1;
            }
            if bytes.get(j) != Some(&b'"') {
                return None; // a raw identifier such as `r#type`
            }
            j += 1;
            while j < bytes.len() {
                let closes = bytes[j] == b'"'
                    && bytes[j + 1..]
                        .iter()
                        .take(hashes)
                        .filter(|b| **b == b'#')
                        .count()
                        == hashes;
                if closes {
                    return Some(j + 1 + hashes);
                }
                j += 1;
            }
            Some(j)
        }
        b'\'' => {
            let next = *bytes.get(i + 1)?;
            if next == b'\\' {
                // The escaped character is at `i + 2` (`'\''` escapes the quote itself),
                // so the closing quote is searched for after it.
                let close = bytes.get(i + 3..)?.iter().position(|b| *b == b'\'')?;
                return Some(i + 4 + close);
            }
            // One character, ASCII or not: `'('`, `'…'`. Anything else is a lifetime.
            let width = match next {
                0x00..=0x7f => 1,
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                _ => 4,
            };
            (bytes.get(i + 1 + width) == Some(&b'\'')).then_some(i + 2 + width)
        }
        _ => None,
    }
}

/// The index just past the comment that starts at `i`, or `None` when none does. Block
/// comments nest, as Rust's do.
fn skip_comment(bytes: &[u8], i: usize) -> Option<usize> {
    match (bytes[i], bytes.get(i + 1)) {
        (b'/', Some(b'/')) => Some(
            bytes[i..]
                .iter()
                .position(|b| *b == b'\n')
                .map_or(bytes.len(), |end| i + end),
        ),
        (b'/', Some(b'*')) => {
            let mut depth = 0usize;
            let mut j = i;
            while j + 1 < bytes.len() {
                match (bytes[j], bytes[j + 1]) {
                    (b'/', b'*') => {
                        depth += 1;
                        j += 2;
                    }
                    (b'*', b'/') => {
                        depth -= 1;
                        j += 2;
                        if depth == 0 {
                            return Some(j);
                        }
                    }
                    _ => j += 1,
                }
            }
            Some(bytes.len())
        }
        _ => None,
    }
}

/// The index of the delimiter that opens a macro's arguments when `name` at `at` is a
/// macro call: `name!(`, `name![` or `name!{`, with any whitespace around the `!`.
fn macro_open(bytes: &[u8], at: usize, name: &str) -> Option<usize> {
    if !bytes[at..].starts_with(name.as_bytes()) {
        return None;
    }
    let after = at + name.len();
    if bytes.get(after).copied().is_some_and(is_ident) {
        return None;
    }
    let mut j = after;
    let skip_space = |mut j: usize| {
        while bytes.get(j).is_some_and(u8::is_ascii_whitespace) {
            j += 1;
        }
        j
    };
    j = skip_space(j);
    if bytes.get(j) != Some(&b'!') {
        return None;
    }
    j = skip_space(j + 1);
    matches!(bytes.get(j), Some(b'(' | b'[' | b'{')).then_some(j)
}

/// Every level macro call in `source`, with its arguments split at the top level. A call
/// nested inside another call's arguments is read too.
fn macro_calls(source: &str) -> Vec<Call> {
    // A span's fields reach a log line too; the `_span` names come first so `info` does not
    // stop at `info_span`.
    const NAMES: &[&str] = &[
        "info_span",
        "warn_span",
        "error_span",
        "debug_span",
        "trace_span",
        "info",
        "warn",
        "error",
        "debug",
        "trace",
        "event",
        "span",
    ];
    // Bytes throughout, never `str` slices: `i` steps one byte at a time and may sit
    // inside a character that is not ASCII.
    let bytes = source.as_bytes();
    let mut calls = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(next) = skip_comment(bytes, i).or_else(|| skip_literal(bytes, i)) {
            i = next;
            continue;
        }
        let found = (i == 0 || !is_ident(bytes[i - 1]))
            .then(|| {
                NAMES
                    .iter()
                    .find_map(|name| macro_open(bytes, i, name).map(|open| (*name, open)))
            })
            .flatten();
        let Some((name, open)) = found else {
            i += 1;
            continue;
        };
        let line = bytes[..i].iter().filter(|b| **b == b'\n').count() + 1;
        let mut depth = 0usize;
        let mut args = Vec::new();
        let mut current = Vec::new();
        let mut j = open;
        while j < bytes.len() {
            if let Some(next) = skip_comment(bytes, j) {
                j = next;
                continue;
            }
            if let Some(next) = skip_literal(bytes, j) {
                let end = next.min(bytes.len());
                current.extend_from_slice(&bytes[j..end]);
                j = next;
                continue;
            }
            let byte = bytes[j];
            match byte {
                b'(' | b'[' | b'{' => {
                    depth += 1;
                    if depth > 1 {
                        current.push(byte);
                    }
                }
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        args.push(String::from_utf8_lossy(&current).trim().to_string());
                        break;
                    }
                    current.push(byte);
                }
                b',' if depth == 1 => {
                    args.push(String::from_utf8_lossy(&current).trim().to_string());
                    current.clear();
                }
                _ => current.push(byte),
            }
            j += 1;
        }
        args.retain(|arg| !arg.is_empty());
        calls.push(Call {
            name: name.to_string(),
            line,
            args,
        });
        // On inside the arguments, so a call nested in one is read as well.
        i = open + 1;
    }
    calls
}

/// Identifiers that name dictation content. A field's value may not mention one, whatever
/// the field is called: `model = %transcript.text` is as much a leak as `text = ...`.
const CONTENT_WORDS: &[&str] = &[
    "text", "raw_text", "partial", "changes", "context", "prompt", "audio", "samples", "pcm",
    "chunk", "written", "spoken", "note", "words", "term", "terms",
];

/// Why `call` breaks DD-93, if it does.
fn problems(call: &Call) -> Vec<String> {
    if call.name == "event" || call.name.contains("span") {
        return vec![format!(
            "`{}!` takes a level or a name before its fields; write the line with a level \
             macro (`info!`, `debug!`, ...) so this test can read it",
            call.name
        )];
    }
    let Some((message, fields)) = call.args.split_last() else {
        return vec!["a log line with no message".to_string()];
    };
    let mut found = Vec::new();
    let fixed = message.starts_with('"') && message.ends_with('"') && !message.contains('{');
    if !fixed {
        found.push(format!(
            "the message must be one string literal with nothing formatted into it, not \
             `{message}`"
        ));
    }
    for field in fields {
        let (name, value) = match field.split_once('=') {
            Some((name, value)) if !value.starts_with('=') => (name.trim(), value.trim()),
            _ => {
                let name = field.trim_start_matches(['%', '?']).trim();
                (name, name)
            }
        };
        if !ALLOWED_FIELDS.iter().any(|(allowed, _)| *allowed == name) {
            found.push(format!(
                "field `{name}` is not in DD-93's list (`ALLOWED_FIELDS`); a field that is \
                 not audio, text, a prompt, a dictionary entry or `context` goes on the list \
                 with the record changed to name it"
            ));
        }
        let mentioned = value
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .find(|word| CONTENT_WORDS.contains(word));
        if let Some(word) = mentioned {
            found.push(format!(
                "field `{name}` is `{value}`, which reads `{word}`: a log line carries no \
                 dictation content, whatever the field is called"
            ));
        }
        if name == "stream" && !value.trim_start_matches(['%', '?']).ends_with("log_id") {
            found.push(format!(
                "`stream` is `{value}`; a log line names a stream by its log id, never the \
                 id that opens it (DD-29)"
            ));
        }
    }
    found
}

#[test]
fn every_dictation_log_line_carries_only_allowed_fields_and_a_fixed_message() {
    let sources = dictation_sources();
    assert!(
        sources.len() > 40,
        "the walk found only {} dictation sources",
        sources.len()
    );
    let mut lines = 0;
    let mut broken = Vec::new();
    for (file, source) in &sources {
        for call in macro_calls(source) {
            lines += 1;
            for problem in problems(&call) {
                broken.push(format!("{file}:{}: {problem}", call.line));
            }
        }
    }
    // The routes, the service and the terminal's installer log today; a reader that finds
    // none is not reading.
    assert!(lines >= 10, "the scan found {lines} log lines");
    assert!(broken.is_empty(), "DD-93:\n{}", broken.join("\n"));
}

#[test]
fn the_exclusions_name_files_that_exist() {
    for (file, why) in NOT_DICTATION_DATA {
        assert!(
            repo_root().join(file).is_file(),
            "{file} ({why}) is gone; drop it from NOT_DICTATION_DATA"
        );
    }
}

/// The reader itself: each rule fails the shape it is for, and comments, strings and
/// lifetimes do not confuse it.
#[test]
fn the_reader_catches_what_it_is_for() {
    let source = r##"
        // tracing::info!(text = %t, "a comment is not a call");
        let s = "info!(text, \"in a string\")";
        let r = r#"debug!(text, "in a raw string")"#;
        fn f<'a>(x: &'a str) { let c = '('; } /* info!(text, "in a block comment") */
        tracing::info!(stream = %stream.log_id, model = %m, "dictation stream opened");
        debug!(text = %transcript.text, "dictation text");
        tracing::warn!(stream = %stream.id, "dictation stream id");
        tracing::info!(model = %m, "dictation {}", transcript.text);
        tracing::info!(model = %m, "dictation {raw_text}");
        tracing::info!(%why, ?backend, segments, "dictation shorthand");
        tracing::event!(tracing::Level::INFO, model = %m, "dictation event");
        let span = tracing::info_span!("dictation", text = %t);
        let ellipsis = '…'; tracing::debug!(model = %m, "after a wide char");
        info![text = %t, "brackets"];
        info!{model = %m, "braces"}
        info ! (model = %transcript.text, "spaced, and content under an allowed name");
        warn!(model = %format!("{}", { warn!(text = %t, "in"); 1 }), "outer");
        let q = ['\'', '"']; info!(model = %m, "after quote chars");
        let b = br#"a"b"#; info!(model = %m, "after a raw byte string");
        info!(model = %m, /* a, comment) */ "with a comment");
        /* outer /* inner */ info!(text, "still a comment") */ info!(model = %m, "after nested");
    "##;
    let calls = macro_calls(source);
    let found: Vec<(usize, Vec<String>)> = calls
        .iter()
        .map(|call| (call.line, problems(call)))
        .collect();
    assert_eq!(found.len(), 18, "{:?}", found);
    // Every problem of every call on `line`; a line must hold at least one call.
    let at = |line: usize| {
        let on_line: Vec<String> = found
            .iter()
            .filter(|(l, _)| *l == line)
            .map(|(_, p)| p.join(" | "))
            .collect();
        assert!(!on_line.is_empty(), "no call read at line {line}");
        on_line.join(" | ")
    };
    assert_eq!(at(6), "", "the allowed line");
    assert!(at(7).contains("field `text`"), "{}", at(7));
    assert!(at(8).contains("never the id"), "{}", at(8));
    assert!(at(9).contains("one string literal"), "{}", at(9));
    assert!(at(10).contains("one string literal"), "{}", at(10));
    assert_eq!(at(11), "", "the shorthands");
    assert!(at(12).contains("level macro"), "{}", at(12));
    assert!(at(13).contains("level macro"), "{}", at(13));
    assert_eq!(at(14), "", "a call after a char literal that is not ASCII");
    assert!(at(15).contains("field `text` is not"), "{}", at(15));
    assert_eq!(at(16), "", "braces");
    assert!(at(17).contains("reads `text`"), "{}", at(17));
    assert!(
        at(18).contains("field `text` is not"),
        "the nested call: {}",
        at(18)
    );
    assert_eq!(at(19), "", "after the quote chars");
    assert_eq!(at(20), "", "after a raw byte string");
    assert_eq!(at(21), "", "a comment inside the arguments");
    assert_eq!(at(22), "", "only the call after the nested comment");
}
