//! **Who opens the dictation history** (DD-96, P-17, P-18).
//!
//! The history (`<local data>/dictation/history/`) holds the text of every dictation a
//! person started. Two rules hold it in place, and this test holds the source to both:
//!
//! 1. **Only the files in [`READERS`] open it**, each with a reason. "Open" is any spelling
//!    in [`OPENERS`]: the shared handle, the daemon's name for it, a handle of one's own
//!    (`HistoryStore::open`), the folder and its name, and the entries file, as a constant
//!    or spelled out in a string. The measured map (file to spelling counts) must equal the
//!    declared one exactly, so a new reader fails here, and so does a row whose reader went
//!    away. The store itself, the leaf crate's `history/` folder ([`THE_STORE`]), is not
//!    measured: it is what is being opened.
//! 2. **Recognition never names it** (P-18). The files and folders in [`NEVER`] are the
//!    dictionary's whole path into a transcript: `bias_set` and the usage counters, the
//!    chat extractor, the scan of recent chats, the observations route, the recognizer and
//!    its learner, the dictionary store, the engine and its worker. Their production code
//!    may not hold a single identifier that contains `history`, nor any opener. Whisper
//!    copies its prompt into the transcript on silence (`docs/dictation/dictionary.md`), so
//!    a prompt built from earlier dictations would put them into new ones.
//!
//! # How it reads the tree
//!
//! Every `.rs` file under `crates/*/src`, with comments and string bodies blanked (the
//! bodies are kept aside, so a path literal is still seen), `#[cfg(test)]` items and test
//! files left out. A mention in a comment is not a read, and a test that opens a history of
//! its own in a temporary folder is not production. The controls below check the reader on
//! a written example and on the store, which certainly spells the file, so a walk that read
//! nothing cannot pass for a clean tree.
//!
//! # What it does not see
//!
//! A path assembled from pieces (`"dictation"` joined with `"history"`), a type renamed by
//! `use ... as`, or a file read through a path that came from somewhere else. Review covers
//! those; this covers the ordinary spellings, which are how a new reader would arrive.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// How one spelling of "open the history" is matched.
#[derive(Clone, Copy)]
enum Spelling {
    /// A whole identifier in code.
    Ident(&'static str),
    /// `Type::item` in code, spaces allowed around `::`.
    Path(&'static str, &'static str),
    /// A string literal that names the entries file.
    FileLiteral,
    /// A string literal that names the folder as a file system path, which is any literal
    /// holding `dictation/history` except a route (`/dictation/history...`), the HTTP
    /// surface `routes::dictation_history` serves.
    FolderLiteral,
}

/// Every spelling that opens the history, by the label the rows use.
const OPENERS: &[(&str, Spelling)] = &[
    ("open_history", Spelling::Ident("open_history")),
    ("shared_history", Spelling::Ident("shared_history")),
    ("HistoryStore::open", Spelling::Path("HistoryStore", "open")),
    ("history_dir", Spelling::Ident("history_dir")),
    ("HISTORY_FOLDER", Spelling::Ident("HISTORY_FOLDER")),
    ("HISTORY_FILE", Spelling::Ident("HISTORY_FILE")),
    ("\"history.jsonl\"", Spelling::FileLiteral),
    ("\"dictation/history\"", Spelling::FolderLiteral),
];

/// The store itself: the leaf crate's module, which decides nothing about privacy and is
/// what every reader below opens.
const THE_STORE: &str = "crates/biorouter-dictation/src/history/";

/// One production file that opens the history, what it spells, and why it may.
struct Reader {
    file: &'static str,
    opens: &'static [(&'static str, usize)],
    why: &'static str,
}

/// Every production file outside [`THE_STORE`] that opens the history.
const READERS: &[Reader] = &[
    Reader {
        file: "crates/biorouter/src/dictation/history.rs",
        opens: &[
            ("\"dictation/history\"", 1),
            ("HISTORY_FOLDER", 2),
            ("HistoryStore::open", 1),
            ("history_dir", 2),
            ("open_history", 1),
        ],
        why: "the core module (DD-96): the folder's name, `history_dir` under the local data \
              folder (never roamed on Windows), and `open_history`, the one handle a process \
              shares and the only `HistoryStore::open` in production",
    },
    Reader {
        file: "crates/biorouter-server/src/routes/dictation_history.rs",
        opens: &[("open_history", 1), ("shared_history", 3)],
        why: "the `/dictation/history` routes, the person's acts on the history (P-17), \
              through `shared_history`, the daemon's name for the shared handle: its \
              definition, `on_history`, which every route's store operation runs through, \
              and the settings answer",
    },
    Reader {
        file: "crates/biorouter-server/src/routes/dictation.rs",
        opens: &[("shared_history", 1)],
        why: "`record_history`, the daemon's one writer of new entries, once per stream a \
              person started (`records_dictation_history`). It appends and never reads an \
              entry back",
    },
    Reader {
        file: "crates/biorouter-server/src/routes/session_reach.rs",
        opens: &[("shared_history", 1)],
        why: "`gate_dictation_history_entry`, which reads the named entry's tier only to \
              answer an id the caller may not list exactly as a missing one",
    },
    Reader {
        file: "crates/biorouter-cli/src/dictation/history_record.rs",
        opens: &[("open_history", 1)],
        why: "the terminal's one recorder (`biorouter dictate` and the terminal front ends), \
              which reads the person's switches and appends",
    },
    Reader {
        file: "crates/biorouter-cli/src/commands/dictation_history.rs",
        opens: &[("open_history", 1)],
        why: "`biorouter dictation history`, the person's acts at a terminal, private entries \
              only at an interactive one with `BIOROUTER_TERMINAL` unset",
    },
    Reader {
        file: "crates/biorouter/src/agents/dictionary_extension.rs",
        opens: &[("open_history", 2)],
        why: "`dictionary__search_dictations` (DD-100), the one reader a model can reach, every \
              entry judged by `agent_reads_dictation` on the call's admitted capability \
              before the limit; and the `use` that brings the handle in",
    },
    Reader {
        file: "crates/biorouter/src/security/dictionary_store.rs",
        opens: &[("history_dir", 2)],
        why: "the path inspector (P-18), which names the folder only to refuse tool reads and \
              writes of it in both tiers, and so never reads an entry",
    },
];

/// The dictionary's path into a transcript, which never names the history (P-18). A
/// trailing `/` names a folder and everything under it.
const NEVER: &[(&str, &str)] = &[
    (
        "crates/biorouter/src/dictation/bias.rs",
        "`bias_set`, the one producer of recognition entries (P-3, DD-36), and `record_usage`",
    ),
    (
        "crates/biorouter/src/dictation/extractor.rs",
        "the chat extractor, which proposes terms from chats",
    ),
    (
        "crates/biorouter/src/dictation/suggestion_scan.rs",
        "Find words in my recent chats, which writes suggestions",
    ),
    (
        "crates/biorouter-server/src/routes/dictionary.rs",
        "the dictionary routes, observations among them; an add from the history arrives as an \
         ordinary `POST /dictionary/entries` the person made (10.6 of the history design)",
    ),
    (
        "crates/biorouter-dictation/src/recognize/",
        "the recognizer: the token prompt, the decoder bonus, the corrections and the \
         observations learner",
    ),
    (
        "crates/biorouter-dictation/src/store/",
        "the dictionary store, the only thing `bias_set` reads",
    ),
    (
        "crates/biorouter-dictation/src/engine/",
        "the engine, which decodes with the prompt `bias_set` built",
    ),
    (
        "crates/biorouter-dictation/src/worker/",
        "the worker process the engine runs in",
    ),
];

// ---------------------------------------------------------------------------------------
// The reader.
// ---------------------------------------------------------------------------------------

/// One file, read: its code lines with comments and string bodies blanked (line numbers
/// kept), and each string literal's body with the line it starts on.
struct Lexed {
    code: Vec<String>,
    literals: Vec<(usize, String)>,
}

/// Blank comments, string bodies and char literals, keeping every line and every brace
/// that is really a brace, and keep each string literal's body aside.
fn lex(src: &str) -> Lexed {
    #[derive(PartialEq)]
    enum S {
        Code,
        Line,
        Block(usize),
        Str,
        Raw(usize),
        Char,
    }
    let b: Vec<char> = src.chars().collect();
    let mut code: Vec<String> = Vec::new();
    let mut literals: Vec<(usize, String)> = Vec::new();
    let mut line = String::new();
    let mut literal = String::new();
    let mut literal_line = 0usize;
    let mut st = S::Code;
    let mut escape = false;
    let mut i = 0usize;
    while i < b.len() {
        let ch = b[i];
        if escape && ch != '\n' {
            escape = false;
            if st == S::Str {
                literal.push(ch);
            }
            line.push(' ');
            i += 1;
            continue;
        }
        escape = false;
        if ch == '\n' {
            code.push(std::mem::take(&mut line));
            if matches!(st, S::Str | S::Raw(_)) {
                literal.push('\n');
            }
            if st == S::Line {
                st = S::Code;
            }
            i += 1;
            continue;
        }
        let two: String = b[i..(i + 2).min(b.len())].iter().collect();
        match st {
            S::Line => {
                line.push(' ');
                i += 1;
            }
            S::Block(depth) => {
                if two == "*/" {
                    st = if depth == 1 {
                        S::Code
                    } else {
                        S::Block(depth - 1)
                    };
                    line.push_str("  ");
                    i += 2;
                } else if two == "/*" {
                    st = S::Block(depth + 1);
                    line.push_str("  ");
                    i += 2;
                } else {
                    line.push(' ');
                    i += 1;
                }
            }
            S::Str => {
                if ch == '\\' {
                    escape = true;
                    literal.push(ch);
                    line.push(' ');
                } else if ch == '"' {
                    st = S::Code;
                    literals.push((literal_line, std::mem::take(&mut literal)));
                    line.push('"');
                } else {
                    literal.push(ch);
                    line.push(' ');
                }
                i += 1;
            }
            S::Raw(hashes) => {
                let closes = ch == '"'
                    && b[i + 1..(i + 1 + hashes).min(b.len())]
                        .iter()
                        .filter(|c| **c == '#')
                        .count()
                        == hashes;
                if closes {
                    st = S::Code;
                    literals.push((literal_line, std::mem::take(&mut literal)));
                    for _ in 0..=hashes {
                        line.push(' ');
                    }
                    i += 1 + hashes;
                } else {
                    literal.push(ch);
                    line.push(' ');
                    i += 1;
                }
            }
            S::Char => {
                if ch == '\\' {
                    escape = true;
                    line.push(' ');
                } else if ch == '\'' {
                    st = S::Code;
                    line.push('\'');
                } else {
                    line.push(' ');
                }
                i += 1;
            }
            S::Code => {
                if two == "//" {
                    st = S::Line;
                    line.push_str("  ");
                    i += 2;
                } else if two == "/*" {
                    st = S::Block(1);
                    line.push_str("  ");
                    i += 2;
                } else if ch == 'r'
                    && i + 1 < b.len()
                    && (b[i + 1] == '"' || b[i + 1] == '#')
                    && (i == 0 || !is_word(b[i - 1]))
                {
                    // `r"…"` or `r#"…"#`; an identifier ending in `r` falls through.
                    let mut j = i + 1;
                    let mut hashes = 0usize;
                    while j < b.len() && b[j] == '#' {
                        hashes += 1;
                        j += 1;
                    }
                    if j < b.len() && b[j] == '"' {
                        st = S::Raw(hashes);
                        literal_line = code.len();
                        for _ in i..=j {
                            line.push(' ');
                        }
                        i = j + 1;
                    } else {
                        line.push(ch);
                        i += 1;
                    }
                } else if ch == '"' {
                    st = S::Str;
                    literal_line = code.len();
                    line.push('"');
                    i += 1;
                } else if ch == '\'' {
                    // A char literal is `'x'` or `'\x'`; anything else is a lifetime.
                    let is_char = (i + 2 < b.len() && b[i + 1] != '\\' && b[i + 2] == '\'')
                        || (i + 3 < b.len() && b[i + 1] == '\\' && b[i + 3] == '\'');
                    st = if is_char { S::Char } else { S::Code };
                    line.push(if is_char { ' ' } else { ch });
                    i += 1;
                } else {
                    line.push(ch);
                    i += 1;
                }
            }
        }
    }
    code.push(line);
    Lexed { code, literals }
}

/// Lines belonging to a `#[cfg(test)]` item, by brace matching over code text.
fn test_region(code: &[String]) -> Vec<bool> {
    let mut skip = vec![false; code.len()];
    let mut i = 0usize;
    while i < code.len() {
        let t = code[i].trim_start();
        if t.starts_with("#[cfg(test)]") || t.starts_with("#[cfg(all(test") {
            let mut j = i;
            let mut depth: i64 = 0;
            let mut opened = false;
            while j < code.len() {
                for ch in code[j].chars() {
                    if ch == '{' {
                        depth += 1;
                        opened = true;
                    } else if ch == '}' {
                        depth -= 1;
                    }
                }
                if opened && depth <= 0 {
                    break;
                }
                if !opened && code[j].trim_end().ends_with(';') {
                    break;
                }
                j += 1;
            }
            for entry in skip.iter_mut().take((j + 1).min(code.len())).skip(i) {
                *entry = true;
            }
            i = j + 1;
            continue;
        }
        i += 1;
    }
    skip
}

/// Is this a production source file? `crates/*/tests/**` are integration tests;
/// `tests.rs`, `*_test.rs`, `*_tests.rs` and `src/**/tests/**` are test modules that happen
/// to live under `src/`.
fn is_production_file(rel: &str) -> bool {
    let Some((_, tail)) = rel.split_once("/src/") else {
        return false;
    };
    if tail.starts_with("tests/") || tail.contains("/tests/") {
        return false;
    }
    let base = tail.rsplit('/').next().unwrap_or(tail);
    !(base == "tests.rs"
        || base.ends_with("_tests.rs")
        || base.ends_with("_test.rs")
        || base.starts_with("tests_"))
}

fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// Whole-identifier occurrences of `ident` in one line of code text.
fn ident_count(line: &str, ident: &str) -> usize {
    let chars: Vec<char> = line.chars().collect();
    let pat: Vec<char> = ident.chars().collect();
    let mut found = 0usize;
    let mut i = 0usize;
    while i + pat.len() <= chars.len() {
        if chars[i..i + pat.len()] == pat[..]
            && (i == 0 || !is_word(chars[i - 1]))
            && (i + pat.len() == chars.len() || !is_word(chars[i + pat.len()]))
        {
            found += 1;
            i += pat.len();
        } else {
            i += 1;
        }
    }
    found
}

/// Occurrences of `ty::item` in one line of code text, spaces around `::` allowed.
fn path_count(line: &str, ty: &str, item: &str) -> usize {
    let mut joined = line.to_string();
    for spaced in [" ::", ":: ", "\t::", "::\t"] {
        while joined.contains(spaced) {
            joined = joined.replace(spaced, "::");
        }
    }
    ident_count(&joined, &format!("{ty}::{item}"))
}

fn names_the_file(literal: &str) -> bool {
    literal.contains("history.jsonl")
}

fn names_the_folder(literal: &str) -> bool {
    literal.contains("dictation/history") && !literal.starts_with("/dictation/history")
}

/// What one file's production text spells, per opener label.
fn openers_in(lexed: &Lexed, production: &[bool]) -> BTreeMap<&'static str, usize> {
    let mut out: BTreeMap<&'static str, usize> = BTreeMap::new();
    for (label, spelling) in OPENERS {
        let n: usize = match spelling {
            Spelling::Ident(ident) => lexed
                .code
                .iter()
                .zip(production)
                .filter(|(_, keep)| **keep)
                .map(|(line, _)| ident_count(line, ident))
                .sum(),
            Spelling::Path(ty, item) => lexed
                .code
                .iter()
                .zip(production)
                .filter(|(_, keep)| **keep)
                .map(|(line, _)| path_count(line, ty, item))
                .sum(),
            Spelling::FileLiteral | Spelling::FolderLiteral => lexed
                .literals
                .iter()
                .filter(|(at, _)| production.get(*at).copied().unwrap_or(false))
                .filter(|(_, body)| match spelling {
                    Spelling::FileLiteral => names_the_file(body),
                    _ => names_the_folder(body),
                })
                .count(),
        };
        if n > 0 {
            out.insert(label, n);
        }
    }
    out
}

/// Production identifiers that contain `history`, any case, as `line: token`.
fn history_names_in(lexed: &Lexed, production: &[bool]) -> Vec<String> {
    let mut out = Vec::new();
    for (index, (line, keep)) in lexed.code.iter().zip(production).enumerate() {
        if !keep {
            continue;
        }
        for token in line.split(|c: char| !is_word(c)) {
            if token.to_ascii_lowercase().contains("history") {
                out.push(format!("{}: {token}", index + 1));
            }
        }
    }
    out
}

/// One production file, read.
struct Source {
    lexed: Lexed,
    /// `true` for each line outside a `#[cfg(test)]` item.
    production: Vec<bool>,
}

fn read_source(src: &str) -> Source {
    let lexed = lex(src);
    let production = test_region(&lexed.code).into_iter().map(|t| !t).collect();
    Source { lexed, production }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Every production `.rs` file under `crates/`, repo relative.
fn scan_tree() -> BTreeMap<String, Source> {
    let root = repo_root();
    let mut tree = BTreeMap::new();
    for entry in walkdir::WalkDir::new(root.join("crates")) {
        let entry = entry.expect("the scan must not silently skip an unreadable directory");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let rel = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if !is_production_file(&rel) {
            continue;
        }
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("the scan could not read {rel}: {e}"));
        let source = read_source(&src);
        assert_eq!(
            source.lexed.code.len(),
            src.split('\n').count(),
            "the reader lost or invented lines in {rel}"
        );
        tree.insert(rel, source);
    }
    tree
}

fn is_under(file: &str, entry: &str) -> bool {
    if entry.ends_with('/') {
        file.starts_with(entry)
    } else {
        file == entry
    }
}

// ---------------------------------------------------------------------------------------
// The guards.
// ---------------------------------------------------------------------------------------

#[test]
fn only_the_listed_files_open_the_dictation_history() {
    let tree = scan_tree();
    let mut measured: BTreeMap<String, BTreeMap<&'static str, usize>> = BTreeMap::new();
    for (rel, source) in &tree {
        if rel.starts_with(THE_STORE) {
            continue;
        }
        let found = openers_in(&source.lexed, &source.production);
        if !found.is_empty() {
            measured.insert(rel.clone(), found);
        }
    }
    let mut declared: BTreeMap<String, BTreeMap<&'static str, usize>> = BTreeMap::new();
    for reader in READERS {
        assert!(
            reader
                .opens
                .iter()
                .all(|(label, n)| *n > 0 && OPENERS.iter().any(|(known, _)| known == label)),
            "{}'s row names an opener that is not in OPENERS, or a count of 0",
            reader.file
        );
        let previous = declared.insert(
            reader.file.to_string(),
            reader.opens.iter().copied().collect(),
        );
        assert!(
            previous.is_none(),
            "{} has two rows; merge them",
            reader.file
        );
        assert!(!reader.why.is_empty(), "{} says no reason", reader.file);
    }
    if measured != declared {
        let mut report = String::new();
        for file in measured.keys().chain(declared.keys()) {
            let (m, d) = (measured.get(file), declared.get(file));
            if m != d && !report.contains(&format!("  {file}\n")) {
                report.push_str(&format!(
                    "  {file}\n    declared: {d:?}\n    measured: {m:?}\n"
                ));
            }
        }
        panic!(
            "the files that open the dictation history have changed:\n\n{report}\n\
             A NEW reader is a new way to reach every dictation the person kept. Before adding \
             its row, say who it serves: the person (behind the history's own gates, P-17), \
             the agent (only through `agent_reads_dictation`, DD-100), or neither, which is a \
             defect. A reader that went AWAY is good news; delete its row so the list stays \
             true. Recognition never gets a row (P-18; see NEVER)."
        );
    }
}

#[test]
fn recognition_never_names_the_dictation_history() {
    let tree = scan_tree();
    let mut failures: Vec<String> = Vec::new();
    for (entry, what) in NEVER {
        let files: Vec<(&String, &Source)> = tree
            .iter()
            .filter(|(rel, _)| is_under(rel, entry))
            .collect();
        assert!(
            !files.is_empty(),
            "{entry} ({what}) is not in the production scan. It moved, and this list must \
             follow it, or the walk read nothing"
        );
        for (rel, source) in files {
            let opens = openers_in(&source.lexed, &source.production);
            if !opens.is_empty() {
                failures.push(format!("  {rel} ({what}) opens it: {opens:?}"));
            }
            let names = history_names_in(&source.lexed, &source.production);
            if !names.is_empty() {
                failures.push(format!("  {rel} ({what}) names it: {names:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "recognition reached for the dictation history (P-18):\n\n{}\n\n\
         History never feeds `bias_set`, the decoder bonus, the corrections, the usage \
         counters, the observations learner or the chat extractor. Whisper copies its prompt \
         into the transcript on silence, so earlier dictations would surface in new ones, and \
         every private rule of the dictionary (tier, institutions, oracles) would need a \
         second spelling for a second source. The person's one bridge is Add to dictionary, \
         an ordinary add they make.",
        failures.join("\n")
    );
    // The NEVER list and the READERS list cannot overlap.
    for reader in READERS {
        assert!(
            !NEVER.iter().any(|(entry, _)| is_under(reader.file, entry)),
            "{} is both a reader and on the recognition side",
            reader.file
        );
    }
}

// ---------------------------------------------------------------------------------------
// The controls. A guard never seen to fail is not known to work.
// ---------------------------------------------------------------------------------------

#[test]
fn the_reader_catches_what_it_is_for() {
    let src = concat!(
        "// open_history() and history_dir() in a comment are not reads\n",
        "/* nor in a block: HistoryStore::open(dir) */\n",
        "use crate::dictation::history::{open_history, HistoryEntry};\n",
        "fn reads(entry: &HistoryEntry) {\n",
        "    let a = open_history();\n",
        "    let file = history_dir().join(\"history.jsonl\");\n",
        "    let b = store :: HistoryStore :: open(dir);\n",
        "    let folder = Paths::in_local_data_dir(\"dictation/history\");\n",
        "    let route = \"/dictation/history/{id}\";\n",
        "    let said = \"open_history() is only a word here\";\n",
        "    let raw = r#\"see history.jsonl\"#;\n",
        "}\n",
        "#[cfg(test)]\n",
        "mod tests {\n",
        "    fn opens_one_of_its_own() { let _ = open_history(); let _ = \"history.jsonl\"; }\n",
        "}\n",
    );
    let source = read_source(src);
    let found = openers_in(&source.lexed, &source.production);
    let want: BTreeMap<&'static str, usize> = [
        ("open_history", 2),
        ("history_dir", 1),
        ("HistoryStore::open", 1),
        ("\"history.jsonl\"", 2),
        ("\"dictation/history\"", 1),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        found, want,
        "the reader miscounted: comments, a route, a word in a string and a test module \
         must not count; the import, the calls, the spaced path and both literals must"
    );
    let names = history_names_in(&source.lexed, &source.production);
    assert_eq!(
        names,
        [
            "3: history",
            "3: open_history",
            "3: HistoryEntry",
            "4: HistoryEntry",
            "5: open_history",
            "6: history_dir",
            "7: HistoryStore",
        ],
        "the recognition check must see every identifier that names the history in code, \
         and none in a comment, a string or a test"
    );
    // A clean recognition file reads as clean.
    let clean =
        read_source("// the history is not read here\nfn bias() { let s = \"history\"; }\n");
    assert!(history_names_in(&clean.lexed, &clean.production).is_empty());
    assert!(openers_in(&clean.lexed, &clean.production).is_empty());
}

#[test]
fn the_scan_reads_the_tree_and_the_store() {
    let tree = scan_tree();
    assert!(
        tree.len() >= 400,
        "only {} production .rs files were scanned; a broken walk reports the same empty set \
         as a clean tree",
        tree.len()
    );
    // The store names its own file, as a constant and as a literal: both detectors see the
    // real tree, not only the example above.
    let store: BTreeMap<&'static str, usize> = tree
        .iter()
        .filter(|(rel, _)| rel.starts_with(THE_STORE))
        .flat_map(|(_, source)| openers_in(&source.lexed, &source.production))
        .fold(BTreeMap::new(), |mut all, (label, n)| {
            *all.entry(label).or_insert(0) += n;
            all
        });
    for label in ["HISTORY_FILE", "\"history.jsonl\""] {
        assert!(
            store.get(label).copied().unwrap_or(0) > 0,
            "the store at {THE_STORE} does not spell {label}, so either it moved or the \
             reader cannot see that spelling: {store:?}"
        );
    }
    // Every reader row names a file the scan read.
    for reader in READERS {
        assert!(
            tree.contains_key(reader.file),
            "{} is not in the production scan; did it move?",
            reader.file
        );
    }
}
