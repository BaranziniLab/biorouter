//! From a command, or a path argument, to a verdict: could the shell reach a
//! path in the secret deny set?
//!
//! [`super::lex`] and [`super::expand`] turn a command into every string each
//! word could become. This module follows the command from start to end,
//! keeping the set of directories it may be in (`cd ~/.aws && head
//! credentials`), the values assigned along the way (`H=$HOME; cat
//! $H/.aws/credentials`) and the scripts it hands to another shell
//! (`bash -c '…'`, a here-document, `eval`), and judges every word against the
//! deny set **by path component, after resolution** — never by the raw token.
//!
//! # Fail closed
//!
//! A path that matches the deny set is refused whether or not it exists. The
//! old guard asked `exists()` of the unexpanded token, which is why `~/…` and
//! `$HOME/…` walked straight through (QA-C H1). Existence is consulted in
//! exactly two places, and in both it can only *add* a refusal: globs are
//! expanded against the real directory (so `cred*` finds `credentials`), and
//! symlinks are canonicalised (so a link into `~/.aws` is judged as `~/.aws`).
//!
//! When part of a path cannot be known before the command runs (a command
//! substitution, a variable nobody set, `cd` into one of those) the path is
//! refused if the directory it would land in holds a protected file, or if its
//! known end could complete a deny pattern.
//!
//! A certain path the scan cannot verify inside its budgets is refused too
//! ([`Reason::TooComplex`]): past [`MAX_CANONICALIZE`] resolutions, a link, a
//! name the disk could spell another way, or a folder never seen before (see
//! `Resolver::physical`).
//!
//! # What this cannot see
//!
//! A path computed at run time by something other than the shell — `python -c`
//! building it from pieces, base64, a script file written earlier — is beyond
//! any static check. That residue is why tool *output* is also scanned for
//! credential material (`biorouter::guardrails::secret_output`). Recursive
//! readers (`grep -r`, `tar`, `cp -r`, `find … -exec cat`) that name only an
//! ancestor directory are the other known gap, for the same reason.

use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use once_cell::sync::Lazy;
use regex::Regex;

use super::expand::{self, Atom, ExpandCtx, ShellEnv, Template};
use super::glob::{self, Tok};
use super::lex::{self, Piece, Token, Word, UNKNOWN};
use super::SecretGuard;

const MAX_CWDS: usize = 16;
const MAX_VAR_VALUES: usize = 16;
const MAX_NESTING: u8 = 4;
const MAX_CANONICALIZE: u32 = 512;
const MAX_LISTINGS: u32 = 64;
const MAX_LISTING_ENTRIES: usize = 4096;
const MAX_GLOB_MATCHES: usize = 1024;
const MAX_WALK_DIRS: usize = 256;
const MAX_WALK_ENTRIES: usize = 8192;
const MAX_WALK_DEPTH: usize = 8;
const MAX_CODE_LITERALS: usize = 2048;

/// The lexical grammars one input has to be judged under, most likely first.
///
/// `\` is a POSIX escape in `sh`/`bash`/`zsh` and an ordinary character — a
/// path separator — in `cmd.exe`/PowerShell. A **Windows host reaches both**:
/// `git-bash`, WSL, MSYS and the coding-agent bridge all hand the Developer
/// server POSIX command lines, and the desktop app runs on the same machine as
/// `cmd.exe`. Choosing one reading by `cfg!(windows)` therefore made the
/// guard's answer a property of the host rather than of the input, and it chose
/// wrongly in the direction that **fails open**: under the Windows reading a
/// POSIX `'\''` splice stays four literal characters, so `sh -c '…sh -c '\''…'`
/// never parses as nesting, [`MAX_NESTING`] is never reached, and a command the
/// guard cannot verify is let through. (Measured as
/// `h1_nesting_past_the_limit_fails_closed` failing on `test (windows-latest)`.)
///
/// So a Windows build judges the input under **both** readings and refuses if
/// **either** lands on a secret — a union, never a choice. Two short-circuits
/// keep that from costing anything measurable:
///
/// * an input with no `\` in it lexes **identically** under both readings —
///   every site in [`super::lex`] that consults the flag is inside a `'\'`
///   match arm — so there is nothing to gain from a second pass, and
/// * the caller stops at the first grammar that refuses, so the second pass
///   runs only when the first already said "allowed".
///
/// How many grammars [`grammars_for`] would run for this input on this host —
/// the observable half of the two short-circuits, so a test can assert the
/// second pass is not paid for when it cannot change the answer.
#[cfg(test)]
pub(crate) fn grammars_for_tests(input: &str, windows_host: bool) -> usize {
    grammars_for(input, windows_host).len()
}

/// A unix build keeps exactly one reading, so its cost is unchanged.
fn grammars(input: &str) -> &'static [bool] {
    grammars_for(input, cfg!(windows))
}

/// [`grammars`] for an explicit host, so the union is exercisable on any
/// machine — the same reason [`super::lex::lex_for`] takes its flag. Without
/// this the Windows-only behaviour would be testable only on Windows, which is
/// how the fail-open survived review in the first place.
fn grammars_for(input: &str, windows_host: bool) -> &'static [bool] {
    if !windows_host {
        return &[false];
    }
    if input.contains('\\') {
        // Host-native reading first: it is the likelier one and, when it
        // refuses, the second pass never runs.
        &[true, false]
    } else {
        &[true]
    }
}

/// Where a relative path is resolved from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Base {
    Dir(PathBuf),
    /// A directory the guard cannot name (after `cd "$(…)"`).
    Unknown,
}

/// Why a scan refused.
#[derive(Debug, Clone)]
pub(crate) struct Finding {
    /// The word as written in the command.
    pub shown: String,
    /// What it resolved to, when it resolved to something.
    pub path: Option<PathBuf>,
    pub reason: Reason,
    /// The word was in a here-document body, read this way (#546). The
    /// refusal says so, because the model wrote the body as text and would
    /// otherwise look for the word among its commands.
    pub heredoc: Option<BodyReading>,
}

/// How the here-document body that held a refused word was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyReading {
    /// As a script this command may run (any body that is not data).
    Script,
    /// As the content of a file ([`data_heredocs`]).
    Data,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reason {
    /// The resolved path matches the deny set.
    Pattern,
    /// Part of the path is unknowable before the command runs, and the
    /// directory it would be in holds a protected file.
    UnresolvedInSecretDirectory,
    /// Part of the path is unknowable, and its known end could complete a deny
    /// pattern (`cd "$(…)" && cat credentials`).
    PatternTail,
    /// Too large or too deeply nested to verify.
    TooComplex,
    /// The command runs `biorouterd browser-key`, the helper only the desktop app may start
    /// (BB-23). See [`HelperSighting`].
    BrowserKeyHelper,
    /// The command starts Biorouter Copilot's engine, which holds the person's Accessibility and
    /// Screen Recording permissions (AD-12). See [`HelperSighting`].
    CopilotEngine,
    /// The command starts an app bundle inside Biorouter's extensions folder, whose helper may
    /// hold the same permissions. See [`Resolver::refuse_extension_app`].
    ExtensionApp,
    /// The command copies, moves or links such a bundle, and a copy keeps those permissions.
    /// See [`Resolver::refuse_copying`].
    ExtensionAppCopy,
}

/// What a scan has seen of the browser-key helper (BB-23), across every word, nested script
/// and code string of one tool call.
///
/// The helper reads the key Chrome encrypts its cookies with, and a person who once answered the
/// macOS Keychain prompt with **Always Allow** has allowed `biorouterd` to read it silently. So a
/// command that runs it is refused in any spelling the scan can read: a path, quotes, `env` and
/// other wrappers, variables, `sh -c` and `eval`, here-documents, an alias body, a command
/// substitution, a string handed to `subprocess` or `system`. It is judged by its words, not its
/// path: the subcommand `browser-key`, together with either the program `biorouterd` (any path,
/// any case, `.exe`) or the `--source` option the helper cannot run without. `browser-key` on its
/// own (a `grep` of this code) is not refused.
///
/// This stops a model that was told to run the helper. It is not what keeps the key from a
/// determined program: the helper itself runs only for Electron main (`browser_key.rs`).
///
/// Biorouter Copilot's engine is the counterpart (AD-12 in docs/design/copilot-agent-desktop.md):
/// the operating system grants Accessibility and Screen Recording to the Biorouter Copilot
/// bundle, so a shell that started it would observe and control the desktop with no Copilot
/// consent. Its executable path inside the bundle or the install's `computer-use` folder is
/// refused on sight; its bundle name or identifier is refused together with a launcher (`open`,
/// `osascript`). A copy of the engine the person installed themselves is not refused: it gets
/// its permissions from whoever starts it, and Biorouter holds none.
#[derive(Default)]
struct HelperSighting {
    subcommand: bool,
    companion: bool,
    copilot_path: bool,
    copilot_name: bool,
    launcher: bool,
}

impl HelperSighting {
    fn note(&mut self, text: &str) {
        let lowered = text.to_lowercase();
        if lowered.contains("copilot.app/contents/macos")
            || lowered.contains("computer-use/agent-desktop")
            || lowered.contains("computer-use\\agent-desktop")
        {
            self.copilot_path = true;
        }
        if lowered.contains("org.biorouter.computer-use")
            || lowered.contains("biorouter copilot")
            || lowered.contains("copilot.app")
        {
            self.copilot_name = true;
        }
        for token in text
            .split(|c: char| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '\\')))
            .filter(|token| !token.is_empty())
        {
            let token = token.to_lowercase();
            if token == "browser-key" {
                self.subcommand = true;
            }
            let program = token.rsplit(['/', '\\']).next().unwrap_or_default();
            if matches!(program, "biorouterd" | "biorouterd.exe") || token == "--source" {
                self.companion = true;
            }
            if matches!(program, "open" | "osascript") {
                self.launcher = true;
            }
        }
    }

    fn seen(&self) -> bool {
        self.subcommand && self.companion
    }

    fn copilot(&self) -> bool {
        self.copilot_path || (self.copilot_name && self.launcher)
    }
}

/// What [`Resolver::denied_resolved`] concluded about one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Clear,
    Denied,
    /// A certain path with a link the budget no longer covers.
    Unverifiable,
}

type Scan = Result<(), Finding>;

#[derive(Clone)]
enum Listing {
    Names(Arc<Vec<String>>),
    /// Not a readable directory: nothing in it can be reached through a glob.
    Unavailable,
    /// Too large, or the listing budget is spent.
    Unverifiable,
}

/// How a component of a path will be treated.
enum Kind {
    Literal(String),
    Glob(Vec<Tok>),
    /// `**`
    Recursive,
    /// Contains a value that cannot be known.
    Hole,
}

enum Root {
    Absolute(PathBuf),
    Relative,
    Unknown,
}

struct Redirect {
    target: Option<Word>,
    body: Option<String>,
    /// The body is data written straight to a file (#546): see
    /// [`data_heredocs`] and [`Resolver::read_data_body`].
    data: bool,
}

pub(crate) struct State {
    cwds: Vec<Base>,
    vars: HashMap<String, Vec<Template>>,
    depth: u8,
}

impl State {
    pub(crate) fn new(bases: &[Base]) -> Self {
        let mut cwds = Vec::new();
        for base in bases {
            if !cwds.contains(base) {
                cwds.push(base.clone());
            }
        }
        if cwds.is_empty() {
            cwds.push(Base::Unknown);
        }
        Self {
            cwds,
            vars: HashMap::new(),
            depth: 0,
        }
    }
}

pub(crate) struct Resolver<'a> {
    guard: &'a SecretGuard,
    env: &'a ShellEnv,
    canonicalize_left: u32,
    listings_left: u32,
    listings: HashMap<PathBuf, Listing>,
    secret_dirs: HashMap<PathBuf, bool>,
    /// A folder as a word named it, mapped to where it really is, recorded
    /// while the symlink budget lasts so that a plain file in a known folder
    /// can still be judged after it runs out (see [`Self::physical`]).
    resolved_dirs: HashMap<PathBuf, PathBuf>,
    /// Whether the word being judged is certainly a path the command uses
    /// (a shell word, a redirect target, a path argument) rather than a
    /// path-looking string pulled out of code. Only a certain path fails closed
    /// when it cannot be verified.
    strict: bool,
    /// The command turned on dot-file globbing (`shopt -s dotglob`, a
    /// `GLOBIGNORE`, `setopt globdots`, a zsh `(D)` qualifier), so `*` reaches
    /// `.env`. Once seen it stays on for the rest of the scan.
    dotglob: bool,
    /// Which reading of `\` this pass is lexing under. Set by
    /// [`Self::under_each_grammar`] for the length of one pass and never read
    /// from the host: see [`grammars`].
    windows_lexing: bool,
    /// Inside a here-document body that is data, not a script this command
    /// runs (#546). Set only by [`Self::read_data_body`], for the length of
    /// that body.
    reading_data: bool,
    /// The browser-key helper's words seen so far in this tool call (BB-23).
    helper: HelperSighting,
    /// The app bundles in Biorouter's extensions folder, found once per scan and only when a
    /// command names an app rather than a path (`open -a`, `open -b`, `osascript`).
    extension_bundles: Option<Vec<ExtensionBundle>>,
}

/// An app bundle found in Biorouter's extensions folder, and the names it may be started by.
struct ExtensionBundle {
    path: PathBuf,
    /// Lower-cased: the bundle's file name without `.app`, and the `CFBundleName`,
    /// `CFBundleDisplayName` and `CFBundleExecutable` an XML `Info.plist` gives.
    names: Vec<String>,
    /// Lower-cased: the `CFBundleIdentifier` an XML `Info.plist` gives.
    identifiers: Vec<String>,
    /// Lower-cased bytes of a binary `Info.plist`, whose identifier is matched by containment.
    binary_plist: Option<Vec<u8>>,
}

/// How deep into the extensions folder bundles are looked for, and how much of it is read.
const MAX_BUNDLE_DEPTH: usize = 6;
const MAX_INFO_PLIST_BYTES: u64 = 256 * 1024;

impl<'a> Resolver<'a> {
    pub(crate) fn new(guard: &'a SecretGuard, env: &'a ShellEnv) -> Self {
        Self {
            guard,
            env,
            canonicalize_left: MAX_CANONICALIZE,
            listings_left: MAX_LISTINGS,
            listings: HashMap::new(),
            secret_dirs: HashMap::new(),
            resolved_dirs: HashMap::new(),
            strict: true,
            dotglob: false,
            windows_lexing: cfg!(windows),
            reading_data: false,
            helper: HelperSighting::default(),
            extension_bundles: None,
        }
    }

    /// Refuse once the call has named the browser-key helper (BB-23) or started Biorouter
    /// Copilot's engine (AD-12).
    fn refuse_helper(&self) -> Scan {
        if self.helper.copilot() {
            return Err(Finding {
                shown: "Biorouter Copilot".to_string(),
                path: None,
                reason: Reason::CopilotEngine,
                heredoc: None,
            });
        }
        if self.helper.seen() {
            return Err(Finding {
                shown: "biorouterd browser-key".to_string(),
                path: None,
                reason: Reason::BrowserKeyHelper,
                heredoc: None,
            });
        }
        Ok(())
    }

    /// Run one scan under every grammar `input` could be read with, refusing if
    /// **any** of them lands on a secret. See [`grammars`] for why this is a
    /// union rather than a choice, and for the two short-circuits.
    fn under_each_grammar(&mut self, input: &str, scan: impl FnMut(&mut Self) -> Scan) -> Scan {
        self.under_grammars(grammars(input), scan)
    }

    fn under_grammars(
        &mut self,
        grammars: &[bool],
        mut scan: impl FnMut(&mut Self) -> Scan,
    ) -> Scan {
        let mut result = Ok(());
        for windows in grammars {
            self.windows_lexing = *windows;
            // Each grammar gets the WHOLE budget, not a share of it. Both
            // budgets fail *closed* for a certain path when they run out (a
            // link `denied_resolved` can no longer resolve, or a directory
            // `list` can no longer read, is refused as too complex) and open
            // for a path-looking string pulled out of code. A second pass
            // running on the drain of the first would therefore answer
            // differently from the first for the same input, which is
            // precisely the asymmetry the union exists to remove. The
            // `listings`, `secret_dirs` and `resolved_dirs` caches are
            // deliberately *not* reset: they memoise facts about the
            // filesystem, which no grammar changes, so the second pass reuses
            // them for free.
            self.canonicalize_left = MAX_CANONICALIZE;
            self.listings_left = MAX_LISTINGS;
            result = scan(self);
            if result.is_err() {
                break;
            }
        }
        result
    }

    /// A shell command line or script, run from any of `bases`.
    pub(crate) fn scan_command(&mut self, command: &str, bases: &[Base]) -> Scan {
        self.scan_command_under(grammars(command), command, bases)
    }

    /// [`Self::scan_command`] against an explicit host's grammar set, so a test
    /// on any machine can ask what a Windows build would do — both what the
    /// Windows reading alone answers (the fail-open) and what the union
    /// answers.
    #[cfg(test)]
    pub(crate) fn scan_command_as_host(
        &mut self,
        command: &str,
        bases: &[Base],
        windows_host: bool,
    ) -> Scan {
        self.scan_command_under(grammars_for(command, windows_host), command, bases)
    }

    /// One grammar, chosen by the caller: the shape the guard had before the
    /// union, so a test can pin what a single reading misses.
    #[cfg(test)]
    pub(crate) fn under_one_grammar_for_tests(
        &mut self,
        windows: bool,
        command: &str,
        bases: &[Base],
    ) -> Scan {
        self.scan_command_under(&[windows], command, bases)
    }

    fn scan_command_under(&mut self, grammars: &[bool], command: &str, bases: &[Base]) -> Scan {
        // The text with the shell's quoting taken out (`bio"router"d`, `browser\-key`), for the
        // spellings no word reading recovers, such as a substitution the expander leaves unknown.
        // It includes a here-document body that is data (#546): the guard still reads such a
        // body, because the file it writes can be run later.
        self.helper
            .note(&command.replace(['\'', '"', '\\', '`'], ""));
        self.under_grammars(grammars, |me| {
            // Which here-documents are data is decided once per grammar, from
            // this pass's own tokens, and everything below uses that one
            // answer: the walk reads those bodies as data, and the two readings
            // of the whole command (the dot-glob detection and the code pass)
            // see the command with those bodies blanked, because the walk has
            // already read them (#546).
            let tokens = lex::lex_for(command, me.windows_lexing);
            let data = data_heredocs(&tokens);
            let outside_data = blank_spans(command, data.iter().map(|(_, span)| span));
            // Before the walk, and for every entry point alike: a test asking
            // what another host would do must not silently lose the dot-glob
            // detection the production path performs.
            if dot_qualifier(&outside_data) {
                me.dotglob = true;
            }
            let data_tokens: Vec<usize> = data.iter().map(|(index, _)| *index).collect();
            let mut state = State::new(bases);
            me.walk_tokens(tokens, &data_tokens, &mut state)?;
            me.scan_code_in(&outside_data, &state)
        })?;
        self.refuse_helper()
    }

    /// A string a tool will treat as a path.
    pub(crate) fn scan_path_value(&mut self, value: &str, bases: &[Base]) -> Scan {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        self.under_each_grammar(trimmed, |me| {
            let state = State::new(bases);
            // As the tool receives it (tools commonly expand a leading `~`)…
            me.check_template(&expand::literal(trimmed), &state, trimmed)?;
            // …and as a shell would read it, for `$HOME/…` and `~user/…`.
            let word = lex_single_word(trimmed, me.windows_lexing);
            me.check_word(&word, &state, trimmed)
        })
    }

    /// Code in some other language (a Ruby or PowerShell script): every
    /// path-looking literal, and every string it hands to a shell.
    pub(crate) fn scan_code(&mut self, text: &str, bases: &[Base]) -> Scan {
        self.under_each_grammar(text, |me| {
            let state = State::new(bases);
            me.scan_code_in(text, &state)
        })?;
        self.refuse_helper()
    }

    // ---- the walk ---------------------------------------------------------

    fn walk(&mut self, script: &str, state: &mut State) -> Scan {
        if state.depth > MAX_NESTING {
            return Err(Finding {
                shown: shorten(script),
                path: None,
                reason: Reason::TooComplex,
                heredoc: None,
            });
        }
        let tokens = lex::lex_for(script, self.windows_lexing);
        self.walk_tokens(tokens, &[], state)
    }

    /// Follow a lexed script. `data` holds the indices, in `tokens`, of the
    /// here-document redirects whose bodies are data ([`data_heredocs`]);
    /// only the top-level command of a scan has any.
    fn walk_tokens(&mut self, tokens: Vec<Token>, data: &[usize], state: &mut State) -> Scan {
        let mut words: Vec<Word> = Vec::new();
        let mut redirects: Vec<Redirect> = Vec::new();
        let mut awaiting_target = false;
        for (index, token) in tokens.into_iter().enumerate() {
            match token {
                Token::Word(word) => {
                    if awaiting_target {
                        if let Some(last) = redirects.last_mut() {
                            last.target = Some(word);
                        }
                        awaiting_target = false;
                    } else {
                        words.push(word);
                    }
                }
                Token::Redirect { op, body, .. } => {
                    let heredoc = op == "<<" || op == "<<-";
                    redirects.push(Redirect {
                        target: None,
                        body,
                        data: heredoc && data.contains(&index),
                    });
                    awaiting_target = !heredoc;
                }
                Token::Op(_) => {
                    awaiting_target = false;
                    self.simple_command(&words, &redirects, state)?;
                    words.clear();
                    redirects.clear();
                }
            }
        }
        self.simple_command(&words, &redirects, state)
    }

    fn nested(&mut self, script: &str, state: &mut State) -> Scan {
        state.depth += 1;
        let result = self.walk(script, state);
        state.depth -= 1;
        result
    }

    fn simple_command(
        &mut self,
        words: &[Word],
        redirects: &[Redirect],
        state: &mut State,
    ) -> Scan {
        // Every word as the shell will pass it: quotes removed, variables and assignments made
        // earlier expanded, an alias body or an `env` prefix included.
        for word in words {
            for reading in self.readings_as_text(word, state) {
                self.helper.note(&reading);
            }
        }
        let mut i = 0;
        while let Some(word) = words.get(i) {
            match plain_text(word).as_deref() {
                Some(
                    "!" | "{" | "}" | "if" | "then" | "else" | "elif" | "fi" | "do" | "done"
                    | "while" | "until" | "esac" | "coproc" | "[[" | "]]" | "function",
                ) => i += 1,
                // bash's `time [-p] pipeline`.
                Some("time") => {
                    i += 1;
                    while matches!(
                        words.get(i).and_then(plain_text).as_deref(),
                        Some("-p" | "--")
                    ) {
                        i += 1;
                    }
                }
                Some("for" | "select") => return self.for_loop(&words[i + 1..], state),
                Some("case") => {
                    if let Some(subject) = words.get(i + 1) {
                        self.check_word(subject, state, &word_text(subject))?;
                    }
                    return Ok(());
                }
                _ => break,
            }
        }
        while let Some((name, value, append)) = words.get(i).and_then(split_assignment) {
            self.assign(&name, &value, append, state)?;
            i += 1;
        }
        let args = &words[i..];
        for (j, word) in args.iter().enumerate() {
            if let Err(whole) = self.check_word(word, state, &word_text(word)) {
                return Err(self.name_inside_script(args, j, whole, state));
            }
        }
        for redirect in redirects {
            if let Some(target) = &redirect.target {
                self.check_word(target, state, &word_text(target))?;
            }
            if let Some(body) = &redirect.body {
                let (read, reading) = if redirect.data {
                    (self.read_data_body(body, state), BodyReading::Data)
                } else {
                    // A here-document is either data or a script (`bash
                    // <<EOF`, `cat <<EOF | sh`, `ssh host <<EOF`). Read it as
                    // both, and as code read from stdin (`python3 - <<EOF`).
                    let read = self
                        .nested(body, state)
                        .and_then(|()| self.scan_code_in(body, state))
                        .and_then(|()| self.scan_code_strings(body, state));
                    (read, BodyReading::Script)
                };
                // The outermost body wins: it is the one the caller wrote.
                read.map_err(|finding| Finding {
                    heredoc: Some(reading),
                    ..finding
                })?;
            }
        }
        if args.is_empty() {
            return Ok(());
        }
        self.program_semantics(args, state)
    }

    /// A quoted here-document body written straight to a file (#546; which
    /// bodies qualify is [`data_heredocs`]).
    ///
    /// The shell hands the body to `cat` as text, so this command neither
    /// expands nor runs anything in it. It is still read as a script and as
    /// code, exactly as any other body is, because a file written here can be
    /// run later, where the guard cannot see inside it: a protected path in
    /// any spelling the resolver knows, a link into a secret store, an unknown
    /// name in a directory that holds a secret, a script nested too deeply and
    /// a link past the symlink budget are all still refused. Two refusals are
    /// dropped, both of them guesses about text this command does not run:
    ///
    /// * the known end of a partly unknown word is not matched against the
    ///   floor ([`Self::tails`]): `res$IHS[is.finite(res$IHS)]` in R and
    ///   `` `…` `` in Markdown are data;
    /// * a literal path that names nothing on disk is not resolved
    ///   ([`Self::denied_resolved`]), so prose does not spend the symlink
    ///   budget that links and real files still answer to.
    ///
    /// The body gets its own copy of the state, one level deeper, as a script
    /// nested anywhere else does, and nothing it changes (a `cd`, a `shopt -s
    /// dotglob`, a zsh `(D)`) reaches the commands around it.
    fn read_data_body(&mut self, body: &str, state: &State) -> Scan {
        let mut local = State {
            cwds: state.cwds.clone(),
            vars: state.vars.clone(),
            depth: state.depth + 1,
        };
        let reading_data = std::mem::replace(&mut self.reading_data, true);
        let dotglob = self.dotglob;
        if dot_qualifier(body) {
            self.dotglob = true;
        }
        let result = self
            .walk(body, &mut local)
            .and_then(|()| self.scan_code_in(body, &local));
        self.reading_data = reading_data;
        self.dotglob = dotglob;
        result
    }

    fn program_semantics(&mut self, args: &[Word], state: &mut State) -> Scan {
        let at = self.program_at(args, state)?;
        self.refuse_extension_app(args, &at, state)?;
        // `env -S 'prog args'`: env splits the string into the program it runs.
        for script in &at.split_strings {
            self.nested(script, state)?;
        }
        let (index, program) = (at.index, at.name);
        let rest = args.get(index + 1..).unwrap_or(&[]);
        match program.as_str() {
            "cd" | "chdir" => self.change_dir(rest, state, true)?,
            "pushd" => self.change_dir(rest, state, false)?,
            "export" | "declare" | "typeset" | "local" | "readonly" => {
                for word in rest {
                    if let Some((name, value, append)) = split_assignment(word) {
                        self.assign(&name, &value, append, state)?;
                    }
                }
            }
            "unset" => {
                for name in rest.iter().filter_map(plain_text) {
                    if is_identifier(&name) {
                        push_var(state, &name, Vec::new());
                    }
                }
            }
            "read" | "mapfile" | "readarray" | "getopts" | "vared" => {
                for name in rest.iter().filter_map(plain_text) {
                    if is_identifier(&name) {
                        push_var(state, &name, vec![Atom::Unknown]);
                    }
                }
            }
            "eval" => {
                for script in self.joined_readings(rest, state) {
                    self.nested(&script, state)?;
                }
            }
            "find" => self.find_names(rest, state)?,
            "shopt" | "setopt" | "set" => {
                let options: Vec<String> = rest
                    .iter()
                    .filter_map(plain_text)
                    .map(|o| o.to_ascii_lowercase().replace('_', ""))
                    .collect();
                if options.iter().any(|o| o == "dotglob" || o == "globdots") {
                    self.dotglob = true;
                }
            }
            _ => {}
        }
        // `sh -c SCRIPT` wherever it appears: `find … -exec sh -c`, `xargs bash
        // -c`, `sudo -u x zsh -c`, `env FOO=1 bash -lc`.
        for (k, word) in args.iter().enumerate() {
            if !is_shell(&program_name(word)) {
                continue;
            }
            if let Some(script) = c_script(&args[k + 1..]) {
                for text in self.readings_as_text(script, state) {
                    self.nested(&text, state)?;
                }
            }
        }
        Ok(())
    }

    // ---- extension apps ---------------------------------------------------

    /// Refuse a command that starts an app bundle inside Biorouter's extensions folder.
    ///
    /// An installed extension may ship a helper app that the person granted macOS Accessibility
    /// and Screen Recording, so a shell that started it would observe and control the desktop
    /// with none of the extension's own consent. Starting means: the program (`args[index]`, the
    /// real one behind `sudo`, `env`, `exec` and the like, or the program `xargs`, `arch` or
    /// `find -exec` runs) is a path inside a bundle there, or `open` opens one (by path, by
    /// `-a` with its name or path, by `-b` with its identifier), or `osascript` names one as an
    /// application. Paths resolve as every other word does: `~`, variables, globs, `cd`, links.
    ///
    /// A copy of a bundle keeps the person's grants (macOS keys them to the code signature, not
    /// the path) and a link to one starts the bundle itself, so `cp`, `mv`, `ln`, `ditto`,
    /// `rsync` and `install` are refused when a source is such a bundle, a program or folder
    /// inside one, or a folder in the extensions folder that holds one
    /// ([`Self::refuse_copying`]).
    ///
    /// Generic on purpose: it names no extension, and it never touches reading or listing,
    /// which are words in no program's place (`ls`, `cat`, `find`, `plutil -p`, `codesign -dv`),
    /// nor `open` on something that does not start ([`Self::refuse_opening`]). It cannot see a
    /// path whose start is unknown until the command runs, a copy made some other way (an
    /// archive, another language) or of a folder above the extensions folder, nor a program
    /// string built in another language.
    fn refuse_extension_app(&mut self, args: &[Word], at: &ProgramAt, state: &State) -> Scan {
        if self.guard.extension_roots().is_empty() {
            return Ok(());
        }
        let Some(word) = args.get(at.index) else {
            return Ok(());
        };
        self.refuse_starting(word, state, true)?;
        // The word read as the program may be the value of an option no table knows: read the
        // next word as the program too, unless the first is a path, which is a program.
        let is_path = plain_text(word).is_some_and(|text| text.chars().any(is_separator));
        if at.after_unknown_option && !is_path {
            if let Some(next) = args.get(at.index + 1) {
                self.refuse_starting(next, state, true)?;
            }
        }
        let rest = &args[at.index + 1..];
        match at.name.as_str() {
            "cp" | "gcp" | "mv" | "gmv" | "ln" | "gln" | "link" | "ditto" | "rsync" | "install"
            | "ginstall" => self.refuse_copying(&at.name, rest, state),
            "open" => self.refuse_opening(rest, state),
            "osascript" => self.refuse_scripted_app(rest, state),
            "xargs" | "arch" => {
                let with_value: &[&str] = if at.name == "xargs" {
                    &[
                        "-n", "-L", "-I", "-P", "-s", "-d", "-E", "-a", "-R", "-S", "-J",
                    ]
                } else {
                    &["-arch", "-e"]
                };
                match program_after_options(rest, with_value) {
                    Some(next) => self.refuse_starting(next, state, true),
                    None => Ok(()),
                }
            }
            "find" => {
                for (k, word) in rest.iter().enumerate() {
                    let flag = plain_text(word).unwrap_or_default();
                    if matches!(flag.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir") {
                        if let Some(next) = rest.get(k + 1) {
                            self.refuse_starting(next, state, true)?;
                        }
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Every path `word` could name now ([`Self::concrete_paths`]), leaving out a reading that
    /// starts with `-` (an option). A word in a program's place (`program`) only counts as a
    /// path when it holds a separator: a bare name is looked up on `PATH`.
    fn word_paths(&mut self, word: &Word, state: &State, program: bool) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for template in self.expand(word, state, false) {
            if matches!(template.first(), Some(Atom::Ch('-', _))) {
                continue;
            }
            if program
                && !template
                    .iter()
                    .any(|atom| matches!(atom, Atom::Ch(c, _) if is_separator(*c)))
            {
                continue;
            }
            out.extend(self.concrete_paths(&template, state));
        }
        out
    }

    /// Refuse `word` when it resolves inside an extension's app bundle.
    fn refuse_starting(&mut self, word: &Word, state: &State, program: bool) -> Scan {
        for path in self.word_paths(word, state, program) {
            if let Some((bundle, _)) = self.in_extension_bundle(&path) {
                return Err(self.finding(&word_text(word), Some(bundle), Reason::ExtensionApp));
            }
        }
        Ok(())
    }

    /// `open`'s operands, and the app its `-a` or `-b` names.
    ///
    /// The app `-a` or `-b` names is started whatever else the command says. An operand is
    /// started only when it is an app bundle (the extension's own, or one nested in it) or a
    /// file that can run (Terminal runs it), or cannot be read: a folder opens in the Finder
    /// and any other file in the app that reads it. `-R` (reveal in the Finder), `-e` and `-t`
    /// (open in a text editor) and `-f` start no operand at all. An operand may be a `file:`
    /// URL, here or as `-u`'s value.
    fn refuse_opening(&mut self, rest: &[Word], state: &State) -> Scan {
        let mut operands: Vec<&Word> = Vec::new();
        let mut starts_no_operand = false;
        let mut i = 0;
        while let Some(word) = rest.get(i) {
            i += 1;
            let text = plain_text(word).unwrap_or_default();
            if text == "--args" {
                break;
            }
            if text.starts_with("--") {
                if matches!(
                    text.as_str(),
                    "--env" | "--stdin" | "--stdout" | "--stderr" | "--arch"
                ) {
                    i += 1;
                }
                continue;
            }
            if text.starts_with('-') && text.len() > 1 {
                if text
                    .chars()
                    .skip(1)
                    .any(|c| matches!(c, 'R' | 'e' | 't' | 'f'))
                {
                    starts_no_operand = true;
                }
                // A flag cluster: the value of its last flag, when that flag takes one, is the
                // next word (`-na Name`, `-b id`).
                let value = rest.get(i);
                match text.chars().last() {
                    Some('a') => {
                        if let Some(value) = value {
                            i += 1;
                            self.refuse_starting(value, state, false)?;
                            self.refuse_named_app(value, state, "open -a")?;
                        }
                    }
                    Some('b') => {
                        if let Some(value) = value {
                            i += 1;
                            self.refuse_identified_app(value, state, "open -b")?;
                        }
                    }
                    Some('u') => {
                        if let Some(value) = value {
                            i += 1;
                            operands.push(value);
                        }
                    }
                    Some('s') => i += 1,
                    _ => {}
                }
                continue;
            }
            operands.push(word);
        }
        if starts_no_operand {
            return Ok(());
        }
        for word in operands {
            let mut paths = self.word_paths(word, state, false);
            paths.extend(
                self.readings_as_text(word, state)
                    .iter()
                    .filter_map(|text| file_url_path(text)),
            );
            for path in paths {
                if let Some((bundle, found)) = self.in_extension_bundle(&path) {
                    if is_app_bundle(&found)
                        || matches!(entry_kind(&found), Entry::Runnable | Entry::Unknown)
                    {
                        return Err(self.finding(
                            &format!("open {}", word_text(word)),
                            Some(bundle),
                            Reason::ExtensionApp,
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// `cp`, `mv`, `ln`, `link`, `ditto`, `rsync` and `install` (and the `g` coreutils
    /// spellings) whose source is an extension's app bundle, or a folder or a file that can run
    /// inside one, or a folder in the extensions folder that holds one. A copy keeps the
    /// person's grants, since macOS keys them to the code signature and not the path, and a
    /// link starts the bundle itself; the start rule cannot follow either, because the copy or
    /// the link does not exist yet when the command is read.
    ///
    /// The sources are every operand but the last (the destination), or every operand when
    /// there is only one (`ln -s bundle`) or a target folder is given (`-t DIR`). An option's
    /// value is read as an operand too, which can only refuse more. A file inside a bundle that
    /// cannot run (an `Info.plist`, a resource) copies as data, and a destination is never
    /// judged: copying into the folder starts nothing.
    fn refuse_copying(&mut self, verb: &str, rest: &[Word], state: &State) -> Scan {
        let gnu_target = matches!(
            verb,
            "cp" | "gcp" | "mv" | "gmv" | "ln" | "gln" | "install" | "ginstall"
        );
        let mut operands: Vec<&Word> = Vec::new();
        let mut target_folder = false;
        let mut options = true;
        for word in rest {
            if let Some(text) = plain_text(word).filter(|_| options) {
                if text == "--" {
                    options = false;
                    continue;
                }
                if let Some(long) = text.strip_prefix("--") {
                    target_folder |= gnu_target && long.starts_with("target-directory");
                    continue;
                }
                if text.starts_with('-') && text.len() > 1 {
                    target_folder |= gnu_target && text.chars().skip(1).any(|c| c == 't');
                    continue;
                }
            }
            operands.push(word);
        }
        let sources = if target_folder || operands.len() == 1 {
            &operands[..]
        } else {
            &operands[..operands.len().saturating_sub(1)]
        };
        for word in sources {
            for path in self.word_paths(word, state, false) {
                let found = match self.in_extension_bundle(&path) {
                    Some((bundle, found)) => (entry_kind(&found) != Entry::Inert).then_some(bundle),
                    None => self.bundle_beneath(&path),
                };
                if let Some(bundle) = found {
                    return Err(self.finding(
                        &format!("{verb} {}", word_text(word)),
                        Some(bundle),
                        Reason::ExtensionAppCopy,
                    ));
                }
            }
        }
        Ok(())
    }

    /// An extension's app bundle inside `path`, when `path` is a folder at or below an
    /// extensions folder (the folder itself, an extension's own folder or one of its
    /// versions), as written or once links are resolved. A folder above the extensions folder
    /// (the config folder, the home folder) is not looked into: copying it is a backup.
    fn bundle_beneath(&mut self, path: &Path) -> Option<PathBuf> {
        let lexical = lexical_normalize(path);
        if let Some(bundle) = self.bundle_beneath_spelling(&lexical) {
            return Some(bundle);
        }
        let physical = self.physical(&lexical)?;
        if physical == lexical {
            return None;
        }
        self.bundle_beneath_spelling(&physical)
    }

    fn bundle_beneath_spelling(&mut self, folder: &Path) -> Option<PathBuf> {
        if !self.guard.in_extension_roots(folder) {
            return None;
        }
        self.extension_bundles()
            .iter()
            .find(|bundle| super::starts_with_folded(&bundle.path, folder))
            .map(|bundle| bundle.path.clone())
    }

    /// An application `osascript -e` names: `application "Name"`, `app "Name"`,
    /// `application id "id"`, or JavaScript's `Application("Name")`.
    fn refuse_scripted_app(&mut self, rest: &[Word], state: &State) -> Scan {
        static NAMED: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r#"(?i)\b(?:application|app)\s*(\bid\s*)?\(?\s*["']([^"']+)["']"#)
                .expect("static regex")
        });
        let mut scripts = Vec::new();
        for (k, word) in rest.iter().enumerate() {
            if plain_text(word).as_deref() == Some("-e") {
                if let Some(script) = rest.get(k + 1) {
                    scripts.extend(self.readings_as_text(script, state));
                }
            }
        }
        for script in scripts {
            for capture in NAMED.captures_iter(&script) {
                let named = capture[2].to_string();
                let shown = format!("osascript {}", capture[0].trim());
                // JavaScript's `Application("…")` takes a name or an identifier alike.
                let found = if capture.get(1).is_some() {
                    self.bundle_with_identifier(&named)
                } else if named.contains('/') {
                    self.in_extension_bundle(Path::new(&named))
                        .map(|(bundle, _)| bundle)
                } else {
                    self.bundle_named(&named)
                        .or_else(|| self.bundle_with_identifier(&named))
                };
                if let Some(bundle) = found {
                    return Err(self.finding(&shown, Some(bundle), Reason::ExtensionApp));
                }
            }
        }
        Ok(())
    }

    fn refuse_named_app(&mut self, word: &Word, state: &State, how: &str) -> Scan {
        for name in self.readings_as_text(word, state) {
            if let Some(bundle) = self.bundle_named(&name) {
                return Err(self.finding(
                    &format!("{how} {}", word_text(word)),
                    Some(bundle),
                    Reason::ExtensionApp,
                ));
            }
        }
        Ok(())
    }

    fn refuse_identified_app(&mut self, word: &Word, state: &State, how: &str) -> Scan {
        for identifier in self.readings_as_text(word, state) {
            if let Some(bundle) = self.bundle_with_identifier(&identifier) {
                return Err(self.finding(
                    &format!("{how} {}", word_text(word)),
                    Some(bundle),
                    Reason::ExtensionApp,
                ));
            }
        }
        Ok(())
    }

    /// The bundle a path is in, as written and once links are resolved, and the spelling of
    /// the path it was found under (the link's target for a link).
    fn in_extension_bundle(&mut self, path: &Path) -> Option<(PathBuf, PathBuf)> {
        let lexical = lexical_normalize(path);
        if let Some(bundle) = self.guard.extension_bundle_of(&lexical) {
            return Some((bundle, lexical));
        }
        let physical = self.physical(&lexical)?;
        let bundle = self.guard.extension_bundle_of(&physical)?;
        Some((bundle, physical))
    }

    /// Every path a word could name, as far as it can be known now: each glob expanded
    /// against the directory it is in, and nothing for a part that cannot be known yet.
    fn concrete_paths(&mut self, template: &Template, state: &State) -> Vec<PathBuf> {
        let mut readings = vec![template.clone()];
        if matches!(template.first(), Some(Atom::Ch('~', _))) {
            let expanded = tilde_at_start(template, self.env);
            if &expanded != template {
                readings.push(expanded);
            }
        }
        let mut out = Vec::new();
        for reading in readings {
            let (root, comps) = split(&reading);
            let starts: Vec<PathBuf> = match root {
                Root::Absolute(prefix) => vec![prefix],
                Root::Relative => state
                    .cwds
                    .iter()
                    .filter_map(|base| match base {
                        Base::Dir(dir) => Some(dir.clone()),
                        Base::Unknown => None,
                    })
                    .collect(),
                Root::Unknown => Vec::new(),
            };
            for start in starts {
                self.concrete_from(start, &comps, &mut out);
            }
        }
        out
    }

    fn concrete_from(&mut self, dir: PathBuf, comps: &[Template], out: &mut Vec<PathBuf>) {
        if out.len() >= MAX_GLOB_MATCHES {
            return;
        }
        let mut cur = dir;
        for (k, comp) in comps.iter().enumerate() {
            match classify(comp) {
                Kind::Literal(name) => push_component(&mut cur, &name),
                Kind::Glob(tokens) => {
                    if let Listing::Names(names) = self.list(&cur) {
                        for name in names.iter() {
                            if glob::matches(&tokens, name, !self.dotglob) {
                                self.concrete_from(cur.join(name), &comps[k + 1..], out);
                            }
                        }
                    }
                    return;
                }
                // The rest of the path is not known yet; what is known so far may already be
                // inside a bundle (`…/Helper.app/**`).
                Kind::Recursive | Kind::Hole => break,
            }
        }
        out.push(cur);
    }

    /// The extension bundles, found on first use.
    fn extension_bundles(&mut self) -> &[ExtensionBundle] {
        if self.extension_bundles.is_none() {
            let found = discover_extension_bundles(self.guard.extension_roots());
            self.extension_bundles = Some(found);
        }
        self.extension_bundles.as_deref().unwrap_or_default()
    }

    /// The bundle `open -a` or `osascript` would start for `name`: a bundle whose file name,
    /// or whose `Info.plist` name, it is (case folded, `.app` optional).
    fn bundle_named(&mut self, name: &str) -> Option<PathBuf> {
        let name = name.trim().trim_end_matches('/').to_lowercase();
        let name = name.strip_suffix(".app").unwrap_or(&name).to_string();
        let tail = name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_string();
        if tail.is_empty() {
            return None;
        }
        self.extension_bundles()
            .iter()
            .find(|bundle| bundle.names.contains(&tail))
            .map(|bundle| bundle.path.clone())
    }

    /// The bundle whose identifier `open -b` or `osascript … id` names.
    fn bundle_with_identifier(&mut self, identifier: &str) -> Option<PathBuf> {
        let identifier = identifier.trim().to_lowercase();
        if identifier.len() < 3 || !identifier.contains('.') {
            return None;
        }
        self.extension_bundles()
            .iter()
            .find(|bundle| {
                bundle.identifiers.contains(&identifier)
                    || bundle.binary_plist.as_ref().is_some_and(|bytes| {
                        bytes
                            .windows(identifier.len())
                            .any(|window| window == identifier.as_bytes())
                    })
            })
            .map(|bundle| bundle.path.clone())
    }

    /// The real program behind wrappers (`sudo`, `env X=1`, `command`, …).
    fn program_of(&mut self, args: &[Word], state: &mut State) -> Result<(usize, String), Finding> {
        let at = self.program_at(args, state)?;
        Ok((at.index, at.name))
    }

    /// [`Self::program_of`], with what the wrappers' options said on the way.
    ///
    /// Each wrapper's options are read as its `getopt` reads them ([`wrapper_spec`]): an option
    /// that takes a value takes the next word with it (`nice -n 10`, `sudo -u root`,
    /// `env -u FOO`, `timeout -s KILL 5`) unless the value is attached (`-n10`,
    /// `--user=root`), in a cluster the first such letter ends the cluster (`sudo -Eu root`),
    /// `--` ends the options, and `timeout`'s duration is one more word before the program.
    /// Until 1.94.6 every option was read as taking no value, so the value was taken for the
    /// program and the real program went unread.
    fn program_at(&mut self, args: &[Word], state: &mut State) -> Result<ProgramAt, Finding> {
        let mut at = ProgramAt {
            index: args.len(),
            name: String::new(),
            after_unknown_option: false,
            split_strings: Vec::new(),
        };
        let mut i = 0;
        while let Some(word) = args.get(i) {
            let name = program_name(word);
            let Some(spec) = wrapper_spec(&name) else {
                at.index = i;
                at.name = name;
                return Ok(at);
            };
            at.after_unknown_option = false;
            i += 1;
            let mut operands = spec.operands;
            let mut options = true;
            while let Some(next) = args.get(i) {
                if let Some((n, value, append)) = split_assignment(next) {
                    self.assign(&n, &value, append, state)?;
                    // Whether or not an unknown option took it as its value, the program
                    // comes after it.
                    at.after_unknown_option = false;
                    i += 1;
                    continue;
                }
                let text = plain_text(next).unwrap_or_default();
                if options && text == "--" {
                    options = false;
                    i += 1;
                    continue;
                }
                if options && text.starts_with('-') {
                    let read = spec.read(&text);
                    at.after_unknown_option = read.unknown;
                    if let Some(split) = read.split {
                        match split {
                            Split::Attached(prefix) => {
                                at.split_strings.extend(
                                    self.readings_as_text(next, state).into_iter().filter_map(
                                        |r| r.strip_prefix(&prefix).map(str::to_string),
                                    ),
                                );
                            }
                            Split::Next => {
                                if let Some(value) = args.get(i + 1) {
                                    at.split_strings.extend(self.readings_as_text(value, state));
                                }
                            }
                        }
                    }
                    i += 1 + usize::from(read.takes_next);
                    continue;
                }
                if operands > 0 {
                    operands -= 1;
                    at.after_unknown_option = false;
                    i += 1;
                    continue;
                }
                break;
            }
        }
        Ok(at)
    }

    /// The refusal for `args[index]`, a word [`Self::check_word`] refused as
    /// one path (`whole`), named for what it really is.
    ///
    /// W1-50: a script handed to a shell (`bash -c 'cat ~/.aws/credentials'`,
    /// `eval '…'`, `find … -exec sh -c '…'`) is also a word of the outer
    /// command, so it was judged as a path before [`Self::program_semantics`]
    /// read it as a script, and the refusal named the whole script resolved
    /// against the working directory ("<cwd>/cat ~/.aws/credentials"). Here the
    /// script is read the way `program_semantics` reads it, and the first path
    /// it refuses, with that path's own resolution, is the one named.
    ///
    /// Only the NAME changes: every arm returns a refusal. When the script
    /// names nothing (it nests too deeply to read, or its text matched only as
    /// a whole), the whole-word refusal stands as the fail-closed fallback,
    /// without a resolution, because a script resolved as a path names no
    /// file. A word that is not a script keeps `whole` exactly.
    fn name_inside_script(
        &mut self,
        args: &[Word],
        index: usize,
        whole: Finding,
        state: &mut State,
    ) -> Finding {
        let Some(word) = args.get(index) else {
            return whole;
        };
        let handed_to_a_shell = args.iter().enumerate().take(index).any(|(k, shell)| {
            is_shell(&program_name(shell))
                && c_script(&args[k + 1..]).is_some_and(|script| std::ptr::eq(script, word))
        });
        let scripts = if handed_to_a_shell {
            self.readings_as_text(word, state)
        } else {
            // `program_of` records `env X=1` assignments; this reading is only
            // for the name, so it works on a copy.
            let mut scratch = State {
                cwds: state.cwds.clone(),
                vars: state.vars.clone(),
                depth: state.depth,
            };
            match self.program_of(args, &mut scratch) {
                Ok((program, name)) if name == "eval" && index > program => {
                    self.joined_readings(&args[program + 1..], &scratch)
                }
                _ => return whole,
            }
        };
        for script in scripts {
            if let Err(inner) = self.nested(&script, state) {
                if inner.reason != Reason::TooComplex {
                    return inner;
                }
            }
        }
        Finding {
            path: None,
            ..whole
        }
    }

    fn change_dir(&mut self, rest: &[Word], state: &mut State, is_cd: bool) -> Scan {
        let target = rest.iter().find(|w| {
            plain_text(w).is_none_or(|t| {
                // Not an option flag like `-L`: `cd -L dir`, `pushd -n dir`.
                let is_flag = t.strip_prefix('-').is_some_and(|after| {
                    !after.is_empty() && !after.chars().all(|c| c.is_ascii_digit())
                });
                !is_flag
            })
        });
        let mut new_bases = Vec::new();
        match target {
            None if is_cd => new_bases.push(match self.env.home() {
                Some(home) => Base::Dir(home.to_path_buf()),
                None => Base::Unknown,
            }),
            // `pushd` alone swaps the top two, both already in the set.
            None => {}
            Some(word) => {
                let text = plain_text(word).unwrap_or_default();
                let stack_index = text
                    .strip_prefix(['+', '-'])
                    .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
                // `cd -` and `pushd +N` return to a directory already in the
                // set, because the set only ever grows.
                if text != "-" && !stack_index {
                    let templates = self.expand(word, state, false);
                    let cdpath: Vec<String> = self
                        .env
                        .get("CDPATH")
                        .map(|c| {
                            c.split(':')
                                .filter(|s| !s.is_empty())
                                .map(String::from)
                                .collect()
                        })
                        .unwrap_or_default();
                    for template in templates {
                        for base in state.cwds.clone() {
                            new_bases.extend(self.resolve_dir(&template, &base));
                        }
                        let rendered = expand::render(&template);
                        if !rendered.starts_with('.') && !rendered.starts_with('/') {
                            for entry in &cdpath {
                                let entry = PathBuf::from(entry);
                                if entry.is_absolute() {
                                    new_bases
                                        .extend(self.resolve_dir(&template, &Base::Dir(entry)));
                                } else {
                                    // A relative CDPATH entry is relative to where `cd` runs.
                                    for base in state.cwds.clone() {
                                        let from = match base {
                                            Base::Dir(dir) => Base::Dir(dir.join(&entry)),
                                            Base::Unknown => Base::Unknown,
                                        };
                                        new_bases.extend(self.resolve_dir(&template, &from));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        for base in new_bases {
            add_cwd(state, base);
        }
        Ok(())
    }

    /// Where `cd TEMPLATE` from `base` may land.
    fn resolve_dir(&mut self, template: &Template, base: &Base) -> Vec<Base> {
        let (root, comps) = split(template);
        let start = match root {
            Root::Unknown => return vec![Base::Unknown],
            Root::Absolute(prefix) => prefix,
            Root::Relative => match base {
                Base::Dir(dir) => dir.clone(),
                Base::Unknown => return vec![Base::Unknown],
            },
        };
        let mut dirs = vec![start];
        for comp in &comps {
            let mut next = Vec::new();
            match classify(comp) {
                Kind::Literal(name) => {
                    for mut dir in dirs {
                        push_component(&mut dir, &name);
                        next.push(dir);
                    }
                }
                Kind::Glob(tokens) => {
                    for dir in dirs {
                        match self.list(&dir) {
                            Listing::Names(names) => {
                                for name in names.iter() {
                                    if glob::matches(&tokens, name, !self.dotglob) {
                                        next.push(dir.join(name));
                                    }
                                }
                            }
                            Listing::Unavailable => {}
                            Listing::Unverifiable => return vec![Base::Unknown],
                        }
                    }
                }
                Kind::Recursive | Kind::Hole => return vec![Base::Unknown],
            }
            if next.len() > MAX_CWDS {
                return vec![Base::Unknown];
            }
            dirs = next;
        }
        dirs.into_iter()
            .map(|d| Base::Dir(lexical_normalize(&d)))
            .collect()
    }

    fn for_loop(&mut self, words: &[Word], state: &mut State) -> Scan {
        let Some(name) = words
            .first()
            .and_then(plain_text)
            .filter(|n| is_identifier(n))
        else {
            return Ok(());
        };
        let mut values = Vec::new();
        if words.get(1).and_then(plain_text).as_deref() == Some("in") {
            for word in &words[2..] {
                // Each list item is a path the body may use: judge it here,
                // globs expanded, whatever the body later does with `$f`.
                self.check_word(word, state, &word_text(word))?;
                values.extend(self.expand(word, state, false));
            }
        } else {
            values.push(vec![Atom::Unknown]);
        }
        for value in values {
            push_var(state, &name, value);
        }
        Ok(())
    }

    fn assign(&mut self, name: &str, value: &Word, append: bool, state: &mut State) -> Scan {
        // Setting GLOBIGNORE turns on bash's dotglob as a side effect.
        if name == "GLOBIGNORE" {
            self.dotglob = true;
        }
        let shown = format!("{name}={}", word_text(value));
        let templates = self.expand(value, state, true);
        for template in &templates {
            self.check_template(template, state, &shown)?;
        }
        if append {
            push_var(state, name, vec![Atom::Unknown]);
        }
        for template in templates {
            push_var(state, name, template);
        }
        Ok(())
    }

    /// `find … -name PATTERN` finds files by a name the command never spells
    /// out as a path. Refuse a pattern that could name a protected file.
    fn find_names(&mut self, rest: &[Word], state: &State) -> Scan {
        let mut i = 0;
        while i < rest.len() {
            let flag = plain_text(&rest[i]).unwrap_or_default();
            if matches!(
                flag.as_str(),
                "-name" | "-iname" | "-path" | "-ipath" | "-wholename" | "-iwholename"
            ) {
                if let Some(value) = rest.get(i + 1) {
                    let shown = format!("{flag} {}", word_text(value));
                    for template in self.expand(value, state, false) {
                        let pattern = expand::literal(&expand::render(&template));
                        let (_, comps) = split(&pattern);
                        self.eval_unknown_root(&comps, &shown)?;
                    }
                }
                i += 2;
            } else {
                i += 1;
            }
        }
        Ok(())
    }

    // ---- words ------------------------------------------------------------

    fn expand(&self, word: &Word, state: &State, assignment: bool) -> Vec<Template> {
        let pwd: Vec<Template> = state
            .cwds
            .iter()
            .map(|b| match b {
                Base::Dir(d) => expand::literal(&d.to_string_lossy()),
                Base::Unknown => vec![Atom::Unknown],
            })
            .collect();
        let ctx = ExpandCtx {
            env: self.env,
            vars: &state.vars,
            pwd: &pwd,
        };
        expand::expand_word(word, &ctx, assignment)
    }

    fn check_word(&mut self, word: &Word, state: &State, shown: &str) -> Scan {
        // Substitutions are commands in their own right: `cat $(echo ~/.aws/credentials)`.
        for piece in word.iter() {
            if let Piece::Subst { source } = piece {
                let mut nested = State {
                    cwds: state.cwds.clone(),
                    vars: state.vars.clone(),
                    depth: state.depth + 1,
                };
                self.walk(source, &mut nested)?;
            }
            if let Piece::Param {
                op: lex::ParamOp::OrDefault(default) | lex::ParamOp::IfSet(default),
                ..
            } = piece
            {
                for inner in default {
                    if let Piece::Subst { source } = inner {
                        let mut nested = State {
                            cwds: state.cwds.clone(),
                            vars: state.vars.clone(),
                            depth: state.depth + 1,
                        };
                        self.walk(source, &mut nested)?;
                    }
                }
            }
        }
        for template in self.expand(word, state, false) {
            if template.is_empty() {
                continue;
            }
            if matches!(template.first(), Some(Atom::Ch('-', _))) {
                // `--config=~/.aws/credentials`: the value is a path.
                if let Some(eq) = template
                    .iter()
                    .position(|a| *a == Atom::Ch('=', true) || *a == Atom::Ch('=', false))
                {
                    let value = tilde_at_start(&template[eq + 1..], self.env);
                    self.check_template(&value, state, shown)?;
                }
                continue;
            }
            self.check_template(&template, state, shown)?;
            if has_code_chars(&template) {
                self.scan_code_in(&expand::render(&template), state)?;
            }
        }
        Ok(())
    }

    fn joined_readings(&self, words: &[Word], state: &State) -> Vec<String> {
        let mut out = vec![String::new()];
        for word in words {
            let readings: Vec<String> = self
                .expand(word, state, false)
                .iter()
                .map(|t| expand::render(t))
                .collect();
            let mut next = Vec::new();
            for prefix in &out {
                for reading in &readings {
                    if next.len() >= expand::MAX_TEMPLATES {
                        break;
                    }
                    let mut joined = prefix.clone();
                    if !joined.is_empty() {
                        joined.push(' ');
                    }
                    joined.push_str(reading);
                    next.push(joined);
                }
            }
            out = next;
        }
        out
    }

    fn readings_as_text(&self, word: &Word, state: &State) -> Vec<String> {
        self.expand(word, state, false)
            .iter()
            .map(|t| expand::render(t))
            .collect()
    }

    /// Path-looking literals in code, and the strings code hands to a shell.
    fn scan_code_in(&mut self, text: &str, state: &State) -> Scan {
        // `subprocess.run(["biorouterd", "browser-key", ...])` names the helper as list items.
        self.helper.note(text);
        static CODE_PATH: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r"[A-Za-z0-9._\-~$+@%:{}*?\[\]]*/[A-Za-z0-9._\-~$+@%:{}*?\[\]/]*")
                .expect("static regex")
        });
        static SHELL_OUT_DQ: Lazy<Regex> = Lazy::new(|| {
            Regex::new(
                r#"(?:do\s+shell\s+script|\b(?:system|popen|shell_exec|passthru|execSync|spawnSync|check_output|check_call|getoutput|getstatusoutput|Popen|subprocess\.run|subprocess\.call)\s*\(\s*[rbuf]?)\s*"((?:[^"\\]|\\.)*)""#,
            )
            .expect("static regex")
        });
        static SHELL_OUT_SQ: Lazy<Regex> = Lazy::new(|| {
            Regex::new(
                r#"\b(?:system|popen|shell_exec|passthru|execSync|spawnSync|check_output|check_call|getoutput|getstatusoutput|Popen|subprocess\.run|subprocess\.call)\s*\(\s*[rbuf]?'((?:[^'\\]|\\.)*)'"#,
            )
            .expect("static regex")
        });

        let was_strict = std::mem::replace(&mut self.strict, false);
        let result = (|| -> Scan {
            for (n, m) in CODE_PATH.find_iter(text).enumerate() {
                if n >= MAX_CODE_LITERALS {
                    break;
                }
                let literal = m.as_str().trim_end_matches(['.', ':']);
                if literal.len() < 2 || literal.contains("://") {
                    continue;
                }
                let word = lex_single_word(literal, self.windows_lexing);
                for template in self.expand(&word, state, false) {
                    self.check_template(&template, state, literal)?;
                }
            }
            Ok(())
        })();
        self.strict = was_strict;
        result?;

        // Ruby, Perl and shell backquotes run their body in a shell.
        static BACKQUOTED: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"`([^`\n]+)`").expect("static regex"));

        if state.depth < MAX_NESTING {
            let mut scripts: Vec<String> = Vec::new();
            for caps in SHELL_OUT_DQ.captures_iter(text).take(64) {
                scripts.push(unescape_code_string(&caps[1]));
            }
            for caps in SHELL_OUT_SQ.captures_iter(text).take(64) {
                scripts.push(unescape_code_string(&caps[1]));
            }
            for caps in BACKQUOTED.captures_iter(text).take(64) {
                scripts.push(caps[1].to_string());
            }
            for script in scripts {
                let mut nested = State {
                    cwds: state.cwds.clone(),
                    vars: state.vars.clone(),
                    depth: state.depth + 1,
                };
                self.walk(&script, &mut nested)?;
            }
        }
        Ok(())
    }

    /// The quoted strings in a here-document body that is not data (#546).
    ///
    /// Code read from stdin (`python3 - <<'EOF'` with `open('.env')`) names a
    /// file by a string with no `/` in it, which [`Self::scan_code_in`] does
    /// not look for and the shell reading of the body sees as one word with
    /// parentheses. Each single- or double-quoted string on one line is judged
    /// as one path, taken literally (no glob, no expansion), the way a code
    /// literal is: refused when it names a protected file, never refused for
    /// being unverifiable. A data body skips this, because writing a file that
    /// mentions `.env` reads nothing.
    fn scan_code_strings(&mut self, text: &str, state: &State) -> Scan {
        static STRING: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r#"'([^'\\\n]*(?:\\.[^'\\\n]*)*)'|"([^"\\\n]*(?:\\.[^"\\\n]*)*)""#)
                .expect("static regex")
        });
        let was_strict = std::mem::replace(&mut self.strict, false);
        let result = (|| -> Scan {
            for caps in STRING.captures_iter(text).take(MAX_CODE_LITERALS) {
                let raw = caps
                    .get(1)
                    .or_else(|| caps.get(2))
                    .map_or("", |m| m.as_str());
                let unescaped = unescape_code_string(raw);
                let literal = unescaped.trim();
                if literal.len() < 2 || literal.contains(['\n', UNKNOWN]) || literal.contains("://")
                {
                    continue;
                }
                let template: Template = literal.chars().map(|c| Atom::Ch(c, false)).collect();
                self.check_template(&template, state, literal)?;
            }
            Ok(())
        })();
        self.strict = was_strict;
        result
    }

    // ---- judging one path -------------------------------------------------

    fn check_template(&mut self, template: &Template, state: &State, shown: &str) -> Scan {
        let mut readings = vec![template.clone()];
        // A `~` that no shell expanded (a value, an option, a code string) may
        // still be expanded by the program that receives it.
        if matches!(template.first(), Some(Atom::Ch('~', _))) {
            let expanded = tilde_at_start(template, self.env);
            if &expanded != template {
                readings.push(expanded);
            }
        }
        for reading in readings {
            let (root, comps) = split(&reading);
            match root {
                Root::Unknown => self.eval_unknown_root(&comps, shown)?,
                Root::Absolute(prefix) => self.eval_path(prefix, &comps, shown)?,
                Root::Relative => {
                    for base in &state.cwds {
                        match base {
                            Base::Dir(dir) => self.eval_path(dir.clone(), &comps, shown)?,
                            Base::Unknown => {
                                let mut with_hole = vec![vec![Atom::Unknown]];
                                with_hole.extend(comps.iter().cloned());
                                self.eval_unknown_root(&with_hole, shown)?;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn eval_path(&mut self, dir: PathBuf, comps: &[Template], shown: &str) -> Scan {
        // The whole path with glob characters and holes taken literally: this
        // alone catches `/Users/*/.aws/credentials`, `~/.ssh/*.pem` and
        // `$X/.aws/credentials`, whatever the directory holds.
        let literal = literalize(&dir, comps);
        if self.guard.is_denied(&lexical_normalize(&literal)) {
            return Err(self.finding(shown, Some(literal), Reason::Pattern));
        }
        self.eval_from(dir, comps, shown)
    }

    fn eval_from(&mut self, dir: PathBuf, comps: &[Template], shown: &str) -> Scan {
        let mut cur = dir;
        for (k, comp) in comps.iter().enumerate() {
            match classify(comp) {
                Kind::Literal(name) => push_component(&mut cur, &name),
                Kind::Glob(tokens) => return self.eval_glob(cur, &tokens, &comps[k + 1..], shown),
                // `**` only recurses when a `/` follows it; at the end it is `*`.
                Kind::Recursive if k + 1 == comps.len() => {
                    return self.eval_glob(cur, &[Tok::Star], &[], shown)
                }
                Kind::Recursive => return self.eval_recursive(cur, &comps[k + 1..], shown),
                Kind::Hole => return self.eval_hole(cur, &comps[k..], shown),
            }
        }
        match self.denied_resolved(&cur) {
            Verdict::Clear => Ok(()),
            Verdict::Denied => Err(self.finding(shown, Some(cur), Reason::Pattern)),
            Verdict::Unverifiable => Err(self.finding(shown, Some(cur), Reason::TooComplex)),
        }
    }

    fn eval_glob(&mut self, dir: PathBuf, tokens: &[Tok], rest: &[Template], shown: &str) -> Scan {
        match self.list(&dir) {
            Listing::Names(names) => {
                let mut matched = 0usize;
                for name in names.iter() {
                    if !glob::matches(tokens, name, !self.dotglob) {
                        continue;
                    }
                    matched += 1;
                    if matched > MAX_GLOB_MATCHES {
                        return self.unexpandable(&dir, rest, shown);
                    }
                    self.eval_from(dir.join(name), rest, shown)?;
                }
                Ok(())
            }
            Listing::Unavailable => Ok(()),
            Listing::Unverifiable => self.unexpandable(&dir, rest, shown),
        }
    }

    /// `**`: zero or more directories, found by a bounded walk that follows
    /// zsh's rules (no dot-directories unless asked, no symlinked directories).
    fn eval_recursive(&mut self, dir: PathBuf, rest: &[Template], shown: &str) -> Scan {
        self.eval_from(dir.clone(), rest, shown)?;
        let dot_ok = self.dotglob
            || rest
                .first()
                .is_some_and(|c| matches!(c.first(), Some(Atom::Ch('.', _))));
        let mut queue = VecDeque::from([(dir.clone(), 0usize)]);
        let (mut dirs_seen, mut entries_seen) = (0usize, 0usize);
        let mut truncated = false;
        while let Some((current, depth)) = queue.pop_front() {
            let Ok(entries) = std::fs::read_dir(&current) else {
                continue;
            };
            dirs_seen += 1;
            let mut names = Vec::new();
            for entry in entries.flatten() {
                entries_seen += 1;
                if entries_seen > MAX_WALK_ENTRIES || dirs_seen > MAX_WALK_DIRS {
                    return self.unexpandable(&dir, rest, shown);
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if is_dir && (dot_ok || !name.starts_with('.')) {
                    if depth + 1 < MAX_WALK_DEPTH {
                        queue.push_back((current.join(&name), depth + 1));
                    } else {
                        truncated = true;
                    }
                }
                names.push(name);
            }
            self.listings
                .insert(current.clone(), Listing::Names(Arc::new(names)));
            if current != dir {
                self.eval_from(current, rest, shown)?;
            }
        }
        if truncated {
            return self.unexpandable(&dir, rest, shown);
        }
        Ok(())
    }

    /// A component holds a value nobody can know yet.
    fn eval_hole(&mut self, dir: PathBuf, comps: &[Template], shown: &str) -> Scan {
        if self.holds_secret(&dir) {
            return Err(self.finding(shown, Some(dir), Reason::UnresolvedInSecretDirectory));
        }
        self.tails(comps, shown)
    }

    /// A glob that could not be expanded: the operator's rule — refuse when the
    /// directory it would expand in holds a protected file — plus the check on
    /// whatever follows it.
    fn unexpandable(&mut self, dir: &Path, rest: &[Template], shown: &str) -> Scan {
        if self.holds_secret(dir) {
            return Err(self.finding(
                shown,
                Some(dir.to_path_buf()),
                Reason::UnresolvedInSecretDirectory,
            ));
        }
        self.tails(rest, shown)
    }

    /// A path whose beginning is unknown.
    fn eval_unknown_root(&mut self, comps: &[Template], shown: &str) -> Scan {
        let literal = literalize(Path::new(""), comps);
        if self.guard.is_denied(&lexical_normalize(&literal)) {
            return Err(self.finding(shown, Some(literal), Reason::Pattern));
        }
        self.tails(comps, shown)
    }

    /// Could the known end of `comps` be the end of a protected path?
    ///
    /// Judged against the built-in floor only. A project's `.biorouterignore`
    /// is also used for build output (`target/`, `*.log`), and treating those
    /// as "could be" matches would refuse `find . -name target`; a project rule
    /// still refuses a *literal* suffix it names, through `is_denied` below.
    fn tails(&mut self, comps: &[Template], shown: &str) -> Scan {
        // (tokens, whether the shell's leading-dot rule applies to them)
        let mut suffix: Vec<(Vec<Tok>, bool)> = Vec::new();
        let mut names: Vec<String> = Vec::new();
        let mut all_literal = true;
        for comp in comps.iter().rev() {
            match classify(comp) {
                Kind::Literal(name) => {
                    if name.is_empty() || name == "." {
                        continue;
                    }
                    let chars: Vec<(char, bool)> = name.chars().map(|c| (c, false)).collect();
                    suffix.push((glob::parse(&chars), true));
                    names.push(name);
                }
                Kind::Glob(tokens) => {
                    suffix.push((tokens, !self.dotglob));
                    all_literal = false;
                }
                Kind::Recursive => break,
                Kind::Hole => {
                    // `$(…).pem`: the known end of a component with a hole. The
                    // hole may start with a dot, so no leading-dot rule.
                    let mut after: Vec<(char, bool)> = comp
                        .iter()
                        .rev()
                        .take_while(|a| !matches!(a, Atom::Unknown))
                        .filter_map(|a| match a {
                            Atom::Ch(c, g) => Some((*c, *g)),
                            Atom::Unknown => None,
                        })
                        .collect();
                    after.reverse();
                    if !after.is_empty() {
                        let mut tokens = vec![Tok::Star];
                        tokens.extend(glob::parse(&after));
                        suffix.push((tokens, false));
                        all_literal = false;
                    }
                    break;
                }
            }
        }
        if suffix.is_empty() {
            return Ok(());
        }
        suffix.reverse();
        names.reverse();
        if all_literal {
            let relative: PathBuf = names.iter().collect();
            if self.guard.is_denied(&relative) {
                return Err(self.finding(shown, Some(relative), Reason::PatternTail));
            }
        }
        if self.reading_data {
            // A guess about where an unknown value lands, in text this
            // command does not run (#546, `Self::read_data_body`).
            return Ok(());
        }
        // A known final name that is a conventional `.env` template (issue #522)
        // cannot end a floor path: the only floor pattern whose last component
        // reaches it is `**/.env.*`, and the template exceptions reopen exactly
        // that name, in any case. Asked of the guard as well, so a
        // `.biorouterignore` that denies the template again still goes on to the
        // comparison below.
        if let Some(name) = suffix.last().and_then(|(tokens, _)| literal_of(tokens)) {
            if is_env_template_name(&name) && !self.guard.is_denied(Path::new(&name)) {
                return Ok(());
            }
        }
        for pattern in floor_components() {
            // Align the two right-anchored: the candidate's known tail against
            // the deny pattern's last components. Whichever is shorter is the
            // overlap that must match end-to-end.
            let (m, k) = (pattern.len(), suffix.len());
            let overlap = m.min(k);
            let suffix_tail = &suffix[k - overlap..];
            let pattern_tail = &pattern[m - overlap..];
            let matches_end =
                suffix_tail
                    .iter()
                    .zip(pattern_tail)
                    .all(|((candidate, dot_rule), denied)| {
                        component_could_match(candidate, *dot_rule, denied)
                    });
            if matches_end {
                return Err(self.finding(shown, None, Reason::PatternTail));
            }
        }
        Ok(())
    }

    // ---- the filesystem ---------------------------------------------------

    /// Denied lexically, or once symlinks are resolved; or, for a certain path
    /// whose links the budget no longer covers, unverifiable.
    fn denied_resolved(&mut self, path: &Path) -> Verdict {
        let lexical = lexical_normalize(path);
        if self.guard.is_denied(&lexical) {
            return Verdict::Denied;
        }
        if self.reading_data && names_nothing(path) {
            // A word of a data body (#546) that names nothing on disk. Asked
            // of the path as written, through the same lookup `open` makes,
            // so a link anywhere on the way, a case-insensitive match or a
            // spelling Windows shortens all find the file and are resolved
            // below like any other path. Only "not found" skips; any other
            // answer is resolved, and fails closed past the budget.
            return Verdict::Clear;
        }
        let Some(physical) = self.physical(path) else {
            // Out of budget with a link, an unusual name or an unknown folder
            // still to resolve. A certain path fails closed (SG-30: 700 benign
            // links and then one to `.netrc` used to read the password); a
            // path-looking string pulled out of code keeps the lexical
            // verdict, as it does for every other budget.
            return if self.strict {
                Verdict::Unverifiable
            } else {
                Verdict::Clear
            };
        };
        if physical != lexical && self.guard.is_denied(&physical) {
            Verdict::Denied
        } else {
            Verdict::Clear
        }
    }

    /// Where `path` really is, or `None` when that would take a symlink
    /// resolution the budget no longer has.
    ///
    /// Inside the budget this is [`canonical_prefix`], exactly as it always
    /// was, and each answer also records where the word's folder really is
    /// (free, unless the word is itself a link). Past the budget a path is
    /// composed from a folder recorded that way, one `lstat` per component and
    /// no resolution at all, which is exact unless something on the way needs
    /// one: a link, or a name the disk could spell differently
    /// ([`is_stable_name`]), because `realpath` is what turns `credentialſ` or
    /// `CREDEN~1` into `credentials` on a case-insensitive or short-name
    /// filesystem. Those, and a path under no folder seen before, answer
    /// `None`.
    fn physical(&mut self, path: &Path) -> Option<PathBuf> {
        if self.canonicalize_left > 0 {
            self.canonicalize_left -= 1;
            let physical = canonical_prefix(path);
            self.remember_folder(path, &physical);
            return Some(physical);
        }
        self.composed(path)
    }

    /// Record where `path`'s folder really is, given where `path` really is.
    /// Only for a path that is not itself a link: a link's resolution names
    /// its target's folder, not its own.
    fn remember_folder(&mut self, path: &Path, physical: &Path) {
        let (Some(_), Some(parent), Some(folder)) =
            (path.file_name(), path.parent(), physical.parent())
        else {
            return;
        };
        if parent.as_os_str().is_empty() || self.resolved_dirs.contains_key(parent) || is_link(path)
        {
            return;
        }
        self.resolved_dirs
            .insert(parent.to_path_buf(), folder.to_path_buf());
    }

    /// `path` from a folder already resolved, past the budget. See
    /// [`Self::physical`].
    fn composed(&mut self, path: &Path) -> Option<PathBuf> {
        let (Some(name), Some(parent)) = (path.file_name(), path.parent()) else {
            return None;
        };
        if parent.as_os_str().is_empty() || !is_stable_name(name) || is_link(path) {
            return None;
        }
        let folder = match self.resolved_dirs.get(parent) {
            Some(folder) => folder.clone(),
            None => {
                let folder = self.composed(parent)?;
                self.resolved_dirs
                    .insert(parent.to_path_buf(), folder.clone());
                folder
            }
        };
        Some(folder.join(name))
    }

    /// Does `dir` directly hold a protected file, or sit under a directory
    /// the user's own `.biorouterignore` protects?
    fn holds_secret(&mut self, dir: &Path) -> bool {
        let dir = lexical_normalize(dir);
        if let Some(known) = self.secret_dirs.get(&dir) {
            return *known;
        }
        if self.guard.is_denied(&dir) {
            self.secret_dirs.insert(dir, true);
            return true;
        }
        let verdict = match self.list(&dir) {
            Listing::Names(names) => names.iter().any(|n| self.guard.is_denied(&dir.join(n))),
            Listing::Unavailable => false,
            // Not cached: it depends on the budget and on `strict`, and a later
            // strict question must not inherit a lenient answer.
            Listing::Unverifiable => return self.strict,
        };
        self.secret_dirs.insert(dir, verdict);
        verdict
    }

    fn list(&mut self, dir: &Path) -> Listing {
        if let Some(listing) = self.listings.get(dir) {
            return listing.clone();
        }
        if self.listings_left == 0 {
            return if self.strict {
                Listing::Unverifiable
            } else {
                Listing::Unavailable
            };
        }
        self.listings_left -= 1;
        let listing = match std::fs::read_dir(dir) {
            Ok(entries) => {
                let mut names = Vec::new();
                let mut too_large = false;
                for entry in entries.flatten() {
                    if names.len() >= MAX_LISTING_ENTRIES {
                        too_large = true;
                        break;
                    }
                    names.push(entry.file_name().to_string_lossy().into_owned());
                }
                if too_large {
                    Listing::Unverifiable
                } else {
                    Listing::Names(Arc::new(names))
                }
            }
            Err(_) => Listing::Unavailable,
        };
        self.listings.insert(dir.to_path_buf(), listing.clone());
        listing
    }

    fn finding(&self, shown: &str, path: Option<PathBuf>, reason: Reason) -> Finding {
        Finding {
            shown: shorten(shown),
            path,
            reason,
            heredoc: None,
        }
    }
}

// ---- helpers ---------------------------------------------------------------

/// A zsh glob qualifier that turns on dot-file matching (`*(D)`, `*(.D)`).
fn dot_qualifier(text: &str) -> bool {
    static DOT_QUALIFIER: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"\([^()|]*D[^()|]*\)").expect("static regex"));
    DOT_QUALIFIER.is_match(text)
}

/// Whether `path` certainly names nothing: `lstat` answers "not found". A
/// relative path is never asked, because the process does not run where the
/// command does.
fn names_nothing(path: &Path) -> bool {
    path.is_absolute()
        && matches!(
            std::fs::symlink_metadata(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        )
}

/// #546: the here-documents of a top-level command whose bodies are data, as
/// `(index of the redirect token, byte span of the body)`.
///
/// A body is data only when the shell will hand it, unexpanded, to a program
/// that writes it to a file and runs nothing, and when nothing else in the
/// same input can run that file. So every simple command in the input has
/// to be exactly `cat > FILE <<'DELIM'` (see [`data_write`]), joined only by
/// newlines, `;`, `&&`, `||` or `&`. One command of any other shape, a pipe
/// or a subshell, and no body in the input is data: `cat > x.sh <<'EOF' …
/// EOF` followed by `bash x.sh` keeps today's reading, as does any receiver
/// the guard cannot classify (`sh`, `python3 -`, `xargs`, `while read`,
/// `at`, `docker exec -i`).
///
/// Only a scan's top-level command is classified. A command nested anywhere
/// (`bash -c`, `eval`, a substitution, another body) may have its output
/// captured or run, so its bodies stay scripts.
fn data_heredocs(tokens: &[Token]) -> Vec<(usize, Range<usize>)> {
    let mut found = Vec::new();
    let mut start = 0;
    for (index, token) in tokens.iter().enumerate() {
        let Token::Op(op) = token else {
            continue;
        };
        if !matches!(*op, "\n" | ";" | "&&" | "||" | "&") {
            return Vec::new();
        }
        if index > start {
            match data_write(&tokens[start..index], start) {
                Some(body) => found.push(body),
                None => return Vec::new(),
            }
        }
        start = index + 1;
    }
    if tokens.len() > start {
        match data_write(&tokens[start..], start) {
            Some(body) => found.push(body),
            None => return Vec::new(),
        }
    }
    found
}

/// One simple command that is exactly bare `cat`, one output redirection to
/// a literal file (`>`, `>>` or `>|`) and one here-document whose delimiter is
/// quoted, read the same way by every POSIX shell, and ended by its line.
/// `offset` is the index of `command[0]` in the whole token list.
fn data_write(command: &[Token], offset: usize) -> Option<(usize, Range<usize>)> {
    let mut program = false;
    let mut output = false;
    let mut heredoc = None;
    let mut awaiting_target = false;
    for (index, token) in command.iter().enumerate() {
        match token {
            Token::Word(word) if awaiting_target => {
                let literal =
                    !word.is_empty() && word.iter().all(|piece| matches!(piece, Piece::Lit { .. }));
                if !literal || output {
                    return None;
                }
                output = true;
                awaiting_target = false;
            }
            Token::Word(word) => {
                let bare_cat = matches!(
                    word.as_slice(),
                    [Piece::Lit { text, quoted: false }] if text == "cat"
                );
                if !bare_cat || program {
                    return None;
                }
                program = true;
            }
            Token::Redirect {
                op,
                heredoc: Some(found),
                ..
            } if op == "<<" || op == "<<-" => {
                let plain_text =
                    found.delimiter_quoted && !found.delimiter_unsupported && found.terminated;
                let span = found.body_span.clone().filter(|_| plain_text)?;
                if heredoc.is_some() || awaiting_target {
                    return None;
                }
                heredoc = Some((offset + index, span));
            }
            Token::Redirect { op, .. } if matches!(op.as_str(), ">" | ">>" | ">|") => {
                if awaiting_target {
                    return None;
                }
                awaiting_target = true;
            }
            _ => return None,
        }
    }
    if awaiting_target || !program || !output {
        return None;
    }
    heredoc
}

/// `text` with each span's characters turned to spaces, newlines kept, so
/// nothing in a span can join text on either side of it into a new word.
fn blank_spans<'t, 'r>(
    text: &'t str,
    spans: impl Iterator<Item = &'r Range<usize>>,
) -> Cow<'t, str> {
    let mut spans: Vec<&Range<usize>> = spans.collect();
    if spans.is_empty() {
        return Cow::Borrowed(text);
    }
    spans.sort_by_key(|span| span.start);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for span in spans {
        let (start, end) = (span.start.max(at), span.end.min(text.len()));
        // A span always comes from lexing `text` itself, so both slices exist;
        // if one did not, the span would stay visible, which only adds
        // refusals.
        let (Some(before), Some(inside)) = (text.get(at..start), text.get(start..end)) else {
            continue;
        };
        out.push_str(before);
        out.extend(inside.chars().map(|c| if c == '\n' { '\n' } else { ' ' }));
        at = end;
    }
    out.push_str(text.get(at..).unwrap_or_default());
    Cow::Owned(out)
}

fn push_var(state: &mut State, name: &str, value: Template) {
    let entry = state.vars.entry(name.to_string()).or_default();
    if entry.contains(&value) {
        return;
    }
    if entry.len() >= MAX_VAR_VALUES {
        let unknown = vec![Atom::Unknown];
        if !entry.contains(&unknown) {
            entry.push(unknown);
        }
        return;
    }
    entry.push(value);
}

fn add_cwd(state: &mut State, base: Base) {
    if state.cwds.contains(&base) {
        return;
    }
    if state.cwds.len() >= MAX_CWDS {
        if !state.cwds.contains(&Base::Unknown) {
            state.cwds.push(Base::Unknown);
        }
        return;
    }
    state.cwds.push(base);
}

/// Where [`Resolver::program_at`] found the program, and what the wrappers before it said.
struct ProgramAt {
    index: usize,
    name: String,
    /// The word right before the program is an option the wrapper's table does not know. It
    /// may take a value, in which case the word read as the program is that value and the real
    /// program is the word after it ([`Resolver::refuse_extension_app`] reads that one too).
    after_unknown_option: bool,
    /// The command lines `env -S` / `--split-string` hands over: `env` splits each one into a
    /// program and its arguments, so each is scanned as a script of its own.
    split_strings: Vec<String>,
}

/// How a wrapper's options read, as its `getopt` (GNU and BSD together) reads them.
struct WrapperSpec {
    /// Short options that take a value.
    short_value: &'static str,
    /// Short options that take none.
    short_flag: &'static str,
    /// Long options (without `--`) that take a value.
    long_value: &'static [&'static str],
    /// Long options that take none, or only an attached one (`--preserve-env=LIST`).
    long_flag: &'static [&'static str],
    /// The option whose value is a command line the wrapper splits (`env -S`).
    split: Option<(char, &'static str)>,
    /// Words after the options and before the program (`timeout`'s duration).
    operands: usize,
}

/// What one option word told [`Resolver::program_at`].
struct OptionRead {
    /// The option's value is the next word.
    takes_next: bool,
    /// The word holds an option the table does not know.
    unknown: bool,
    /// The word is the split-string option, with its value attached or in the next word.
    split: Option<Split>,
}

enum Split {
    /// The value is the word's text after this prefix (`-S'cmd'`, `--split-string=cmd`).
    Attached(String),
    /// The value is the next word.
    Next,
}

impl WrapperSpec {
    /// Read one word that starts with `-`.
    fn read(&self, text: &str) -> OptionRead {
        let mut read = OptionRead {
            takes_next: false,
            unknown: false,
            split: None,
        };
        if let Some(long) = text.strip_prefix("--") {
            let (written, attached) = match long.split_once('=') {
                Some((written, _)) => (written, true),
                None => (long, false),
            };
            // getopt_long takes the full name or an unambiguous prefix of it.
            let all = || self.long_value.iter().chain(self.long_flag.iter()).copied();
            let full: Option<&'static str> = all().find(|name| *name == written).or_else(|| {
                let mut prefixed =
                    all().filter(|name| !written.is_empty() && name.starts_with(written));
                match (prefixed.next(), prefixed.next()) {
                    (Some(name), None) => Some(name),
                    _ => None,
                }
            });
            let Some(full) = full else {
                read.unknown = true;
                return read;
            };
            if self.long_value.contains(&full) {
                read.takes_next = !attached;
                if self.split.is_some_and(|(_, split)| split == full) {
                    read.split = Some(if attached {
                        Split::Attached(format!("--{written}="))
                    } else {
                        Split::Next
                    });
                }
            }
            return read;
        }
        let cluster = text.get(1..).unwrap_or_default();
        // `nice -10`, the old spelling of `-n 10`; and a lone `-` (`env -`).
        if cluster.chars().all(|c| c.is_ascii_digit()) {
            return read;
        }
        for (at, c) in cluster.char_indices() {
            if self.short_value.contains(c) {
                let rest = cluster.get(at + c.len_utf8()..).unwrap_or_default();
                read.takes_next = rest.is_empty();
                if self.split.is_some_and(|(short, _)| short == c) {
                    read.split = Some(if rest.is_empty() {
                        Split::Next
                    } else {
                        Split::Attached(
                            text.get(..1 + at + c.len_utf8())
                                .unwrap_or_default()
                                .to_string(),
                        )
                    });
                }
                return read;
            }
            if !self.short_flag.contains(c) {
                read.unknown = true;
            }
        }
        read
    }
}

// The long options of the wrappers below, GNU and BSD together.
const TIME_LONG_FLAG: &[&str] = &[
    "append",
    "portability",
    "quiet",
    "verbose",
    "help",
    "version",
];
const SUDO_LONG_VALUE: &[&str] = &[
    "auth-type",
    "chdir",
    "chroot",
    "close-from",
    "command-timeout",
    "group",
    "host",
    "login-class",
    "other-user",
    "prompt",
    "role",
    "type",
    "user",
];
const SUDO_LONG_FLAG: &[&str] = &[
    "askpass",
    "background",
    "bell",
    "edit",
    "help",
    "list",
    "login",
    "no-update",
    "non-interactive",
    "preserve-env",
    "preserve-groups",
    "remove-timestamp",
    "reset-timestamp",
    "set-home",
    "shell",
    "stdin",
    "validate",
    "version",
];
const ENV_LONG_FLAG: &[&str] = &[
    "block-signal",
    "debug",
    "default-signal",
    "help",
    "ignore-environment",
    "ignore-signal",
    "list-signal-handling",
    "null",
    "version",
];
const TIMEOUT_LONG_FLAG: &[&str] = &[
    "foreground",
    "preserve-status",
    "verbose",
    "help",
    "version",
];
const HELP: &[&str] = &["help", "version"];

/// The wrappers [`Resolver::program_at`] looks through, and how their options read. A name
/// not here is the program.
fn wrapper_spec(name: &str) -> Option<WrapperSpec> {
    const fn spec(
        short_value: &'static str,
        short_flag: &'static str,
        long_value: &'static [&'static str],
        long_flag: &'static [&'static str],
    ) -> WrapperSpec {
        WrapperSpec {
            short_value,
            short_flag,
            long_value,
            long_flag,
            split: None,
            operands: 0,
        }
    }
    Some(match name {
        "builtin" | "nohup" | "noglob" => spec("", "", &[], HELP),
        "command" => spec("", "pvV", &[], &[]),
        // bash's `exec [-cl] [-a name]`.
        "exec" => spec("a", "cl", &[], &[]),
        // `/usr/bin/time`; the `time` keyword is read in `simple_command`.
        "time" => spec("fo", "ahlpqvV", &["format", "output"], TIME_LONG_FLAG),
        "nice" => spec("n", "", &["adjustment"], HELP),
        "sudo" => spec(
            "aCcDgpRrTtUu",
            "ABbEeHhiKklNnPSsVv",
            SUDO_LONG_VALUE,
            SUDO_LONG_FLAG,
        ),
        "doas" => spec("aCu", "Lns", &[], &[]),
        // `env -S STRING` splits STRING into the program and its arguments.
        "env" => WrapperSpec {
            split: Some(('S', "split-string")),
            ..spec(
                "CLPSUu",
                "0iv",
                &["chdir", "split-string", "unset"],
                ENV_LONG_FLAG,
            )
        },
        "caffeinate" => spec("tw", "dimsu", &[], &[]),
        "stdbuf" => spec("eio", "", &["error", "input", "output"], HELP),
        // `timeout [options] DURATION COMMAND`.
        "timeout" => WrapperSpec {
            operands: 1,
            ..spec("ks", "v", &["kill-after", "signal"], TIMEOUT_LONG_FLAG)
        },
        "unbuffer" => spec("", "p", &[], &[]),
        _ => return None,
    })
}

/// The program a runner such as `xargs` or `arch` starts: its first word after its options,
/// the options in `with_value` taking the next word as their value.
fn program_after_options<'w>(rest: &'w [Word], with_value: &[&str]) -> Option<&'w Word> {
    let mut i = 0;
    while let Some(word) = rest.get(i) {
        let text = plain_text(word).unwrap_or_default();
        if text == "--" {
            return rest.get(i + 1);
        }
        if text.starts_with('-') && text.len() > 1 {
            i += if with_value.contains(&text.as_str()) {
                2
            } else {
                1
            };
            continue;
        }
        return Some(word);
    }
    None
}

/// What is at a path inside an extension's bundle, as far as starting it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    Folder,
    /// A file that can run: on Unix, one with an execute bit.
    Runnable,
    /// A file that cannot run (an `Info.plist`, a resource).
    Inert,
    /// Nothing can be read there, which counts as able to run.
    Unknown,
}

fn entry_kind(path: &Path) -> Entry {
    match std::fs::metadata(path) {
        Err(_) => Entry::Unknown,
        Ok(meta) if meta.is_dir() => Entry::Folder,
        Ok(meta) if can_run(&meta) => Entry::Runnable,
        Ok(_) => Entry::Inert,
    }
}

#[cfg(unix)]
fn can_run(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

/// No execute bit to read; the grants this rule protects are macOS's.
#[cfg(not(unix))]
fn can_run(_: &std::fs::Metadata) -> bool {
    false
}

/// Whether `path` names an app bundle itself (its last component is `*.app`).
fn is_app_bundle(path: &Path) -> bool {
    path.file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .is_some_and(|name| name.len() > 4 && name.ends_with(".app"))
}

/// The local path a `file:` URL names (`file:///…`, `file://localhost/…`), percent-decoded.
fn file_url_path(text: &str) -> Option<PathBuf> {
    let scheme = text.get(..7)?;
    if !scheme.eq_ignore_ascii_case("file://") {
        return None;
    }
    let mut rest = text.get(7..)?;
    if rest
        .get(..9)
        .is_some_and(|host| host.eq_ignore_ascii_case("localhost"))
    {
        rest = rest.get(9..).unwrap_or_default();
    }
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                decoded.push(byte);
                i += 3;
            }
            (byte, _) => {
                decoded.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(decoded).ok().map(PathBuf::from)
}

/// Every `*.app` bundle under the extensions folders, within the same walk budgets as `**`.
/// A bundle is not walked into, and links are not followed.
fn discover_extension_bundles(roots: &[PathBuf]) -> Vec<ExtensionBundle> {
    let mut bundles: Vec<ExtensionBundle> = Vec::new();
    let mut queue: VecDeque<(PathBuf, usize)> =
        roots.iter().map(|root| (root.clone(), 0)).collect();
    let (mut dirs_seen, mut entries_seen) = (0usize, 0usize);
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        dirs_seen += 1;
        if dirs_seen > MAX_WALK_DIRS {
            break;
        }
        for entry in entries.flatten() {
            entries_seen += 1;
            if entries_seen > MAX_WALK_ENTRIES {
                return bundles;
            }
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if let Some(stem) = name.strip_suffix(".app").filter(|stem| !stem.is_empty()) {
                if bundles.iter().any(|known| known.path == path) {
                    continue;
                }
                bundles.push(read_bundle(path, stem));
            } else if depth + 1 < MAX_BUNDLE_DEPTH {
                queue.push_back((path, depth + 1));
            }
        }
    }
    bundles
}

/// A bundle's names and identifier from its `Info.plist`, when it can be read.
fn read_bundle(path: PathBuf, stem: &str) -> ExtensionBundle {
    static KEYED: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"<key>\s*(CFBundleIdentifier|CFBundleName|CFBundleDisplayName|CFBundleExecutable)\s*</key>\s*<string>\s*([^<]+?)\s*</string>",
        )
        .expect("static regex")
    });
    let mut bundle = ExtensionBundle {
        names: vec![stem.to_string()],
        identifiers: Vec::new(),
        binary_plist: None,
        path,
    };
    let plist = bundle.path.join("Contents").join("Info.plist");
    let bytes = std::fs::File::open(&plist).ok().and_then(|file| {
        use std::io::Read;
        let mut bytes = Vec::new();
        file.take(MAX_INFO_PLIST_BYTES)
            .read_to_end(&mut bytes)
            .ok()
            .map(|_| bytes)
    });
    let Some(bytes) = bytes else {
        return bundle;
    };
    if bytes.starts_with(b"bplist") {
        bundle.binary_plist = Some(bytes.to_ascii_lowercase());
        return bundle;
    }
    let text = String::from_utf8_lossy(&bytes);
    for capture in KEYED.captures_iter(&text) {
        let value = capture[2].to_lowercase();
        if &capture[1] == "CFBundleIdentifier" {
            bundle.identifiers.push(value);
        } else if !bundle.names.contains(&value) {
            bundle.names.push(value);
        }
    }
    bundle
}

/// A word with no expansions, as text.
fn plain_text(word: &Word) -> Option<String> {
    let mut out = String::new();
    for piece in word {
        match piece {
            Piece::Lit { text, .. } => out.push_str(text),
            _ => return None,
        }
    }
    Some(out)
}

/// A word as the user wrote it, near enough for a message.
fn word_text(word: &Word) -> String {
    word.iter()
        .map(|piece| match piece {
            Piece::Lit { text, .. } => text.clone(),
            Piece::Param { name, .. } => format!("${name}"),
            Piece::Subst { source } => format!("$({source})"),
            Piece::Unknown => UNKNOWN.to_string(),
        })
        .collect()
}

fn program_name(word: &Word) -> String {
    plain_text(word)
        .map(|t| {
            t.rsplit(['/', '\\'])
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase()
        })
        .unwrap_or_default()
}

fn is_shell(name: &str) -> bool {
    let name = name.strip_suffix(".exe").unwrap_or(name);
    matches!(
        name,
        "sh" | "bash"
            | "zsh"
            | "dash"
            | "ksh"
            | "mksh"
            | "ash"
            | "yash"
            | "fish"
            | "csh"
            | "tcsh"
            | "rbash"
    )
}

/// After a shell's name: the script given to `-c` (in any flag cluster).
fn c_script(after: &[Word]) -> Option<&Word> {
    let mut saw_c = false;
    let mut words = after.iter();
    while let Some(word) = words.next() {
        let text = plain_text(word);
        match text.as_deref() {
            // `-o pipefail`, `+O extglob`: an option that takes a value.
            Some("-o" | "+o" | "-O" | "+O") => {
                words.next();
            }
            Some(t)
                if (t.starts_with('-') || t.starts_with('+'))
                    && t.len() > 1
                    && !t.starts_with("--") =>
            {
                // A short-flag cluster containing `c`: `-lc`, `-ic`.
                if t.get(1..).is_some_and(|flags| flags.contains('c')) {
                    saw_c = true;
                }
            }
            Some(t) if t.starts_with("--") => {}
            _ if saw_c => return Some(word),
            _ => return None,
        }
    }
    None
}

/// Could one component of a path match one component of a floor pattern?
///
/// Decided only where one side is literal. Two wildcards are not compared:
/// `*.py` and `secrets.*` share `secrets.py`, and refusing on that would refuse
/// every glob by extension. A wildcard floor pattern is still enforced against
/// a glob's *literal text* (`*.pem` is itself a name `**/*.pem` matches) by the
/// literal check every path gets first.
fn component_could_match(candidate: &[Tok], dot_rule: bool, denied: &[Tok]) -> bool {
    if let Some(name) = literal_of(candidate) {
        return glob::matches(denied, &name, false);
    }
    if let Some(name) = literal_of(denied) {
        return glob::matches(candidate, &name, dot_rule);
    }
    false
}

fn literal_of(tokens: &[Tok]) -> Option<String> {
    tokens
        .iter()
        .map(|t| match t {
            Tok::Lit(c) => Some(*c),
            _ => None,
        })
        .collect()
}

/// Whether `name` is one of the `.env` template names the floor reopens
/// ([`super::DEFAULT_SECRET_TEMPLATE_EXCEPTIONS`]), in any case, for
/// [`Resolver::tails`] and [`super::SecretGuard::is_denied`].
pub(super) fn is_env_template_name(name: &str) -> bool {
    static TEMPLATES: Lazy<Vec<Vec<Tok>>> = Lazy::new(|| {
        super::DEFAULT_SECRET_TEMPLATE_EXCEPTIONS
            .iter()
            .map(|pattern| {
                glob::parse_pattern(pattern.trim_start_matches('!').trim_start_matches("**/"))
            })
            .collect()
    });
    TEMPLATES
        .iter()
        .any(|template| glob::matches(template, name, false))
}

/// The floor's patterns as component globs, for [`Resolver::tails`].
fn floor_components() -> &'static [Vec<Vec<Tok>>] {
    static FLOOR: Lazy<Vec<Vec<Vec<Tok>>>> = Lazy::new(|| {
        super::DEFAULT_SECRET_PATTERNS
            .iter()
            .map(|pattern| {
                pattern
                    .trim_start_matches("**/")
                    .split('/')
                    .map(glob::parse_pattern)
                    .collect()
            })
            .collect()
    });
    &FLOOR
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `NAME=value`, `NAME+=value`, `NAME[i]=value` at the start of a word.
fn split_assignment(word: &Word) -> Option<(String, Word, bool)> {
    let Some(Piece::Lit {
        text,
        quoted: false,
    }) = word.first()
    else {
        return None;
    };
    let (head, rest) = text.split_once('=')?;
    let (name, append) = match head.strip_suffix('+') {
        Some(n) => (n, true),
        None => (head, false),
    };
    let name = name.split('[').next().unwrap_or_default();
    if !is_identifier(name) {
        return None;
    }
    let mut value: Word = Vec::new();
    if !rest.is_empty() {
        value.push(Piece::Lit {
            text: rest.to_string(),
            quoted: false,
        });
    }
    value.extend(word[1..].iter().cloned());
    Some((name.to_string(), value, append))
}

fn lex_single_word(text: &str, windows: bool) -> Word {
    match lex::lex_for(text, windows).into_iter().next() {
        Some(Token::Word(word)) => word,
        _ => vec![Piece::Lit {
            text: text.to_string(),
            quoted: false,
        }],
    }
}

fn has_code_chars(template: &Template) -> bool {
    template.iter().any(|a| {
        matches!(
            a,
            Atom::Ch(
                ' ' | '\t' | '\n' | '(' | ')' | '\'' | '"' | ';' | ',' | '+' | '`' | '|' | '&',
                _
            )
        )
    })
}

fn unescape_code_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Expand a leading `~` or `~/` that no shell expanded.
fn tilde_at_start(template: &[Atom], env: &ShellEnv) -> Template {
    let is_sep = |a: Option<&Atom>| matches!(a, None | Some(Atom::Ch('/', _)));
    if matches!(template.first(), Some(Atom::Ch('~', _))) && is_sep(template.get(1)) {
        if let Some(home) = env.home() {
            let mut out: Template = home
                .to_string_lossy()
                .chars()
                .map(|c| Atom::Ch(c, false))
                .collect();
            out.extend(template[1..].iter().cloned());
            return out;
        }
    }
    template.to_vec()
}

fn is_separator(c: char) -> bool {
    c == '/' || (cfg!(windows) && c == '\\')
}

/// Split a template into its root and its components.
fn split(template: &Template) -> (Root, Vec<Template>) {
    let mut comps: Vec<Template> = vec![Vec::new()];
    for atom in template {
        match atom {
            Atom::Ch(c, _) if is_separator(*c) => comps.push(Vec::new()),
            other => comps.last_mut().expect("never empty").push(other.clone()),
        }
    }
    if matches!(template.first(), Some(Atom::Unknown)) {
        return (Root::Unknown, comps);
    }
    if matches!(template.first(), Some(Atom::Ch(c, _)) if is_separator(*c)) {
        comps.remove(0);
        #[cfg(windows)]
        if matches!(template.get(1), Some(Atom::Ch(c, _)) if is_separator(*c)) {
            // `\\server\share\…`
            let server = comps.get(1).map(|c| expand::render(c)).unwrap_or_default();
            let share = comps.get(2).map(|c| expand::render(c)).unwrap_or_default();
            let rest = comps.split_off(3.min(comps.len()));
            return (
                Root::Absolute(PathBuf::from(format!(r"\\{server}\{share}\"))),
                rest,
            );
        }
        return (
            Root::Absolute(PathBuf::from(std::path::MAIN_SEPARATOR_STR)),
            comps,
        );
    }
    #[cfg(windows)]
    {
        // `C:\…` / `C:/…`
        if let (Some(Atom::Ch(letter, _)), Some(Atom::Ch(':', _))) =
            (template.first(), template.get(1))
        {
            if letter.is_ascii_alphabetic()
                && matches!(template.get(2), Some(Atom::Ch(c, _)) if is_separator(*c))
            {
                let mut rest = comps;
                rest.remove(0);
                return (Root::Absolute(PathBuf::from(format!("{letter}:\\"))), rest);
            }
        }
    }
    (Root::Relative, comps)
}

fn classify(comp: &Template) -> Kind {
    if comp.iter().any(|a| matches!(a, Atom::Unknown)) {
        return Kind::Hole;
    }
    let active = comp
        .iter()
        .any(|a| matches!(a, Atom::Ch('*' | '?' | '[', true)));
    if !active {
        return Kind::Literal(expand::render(comp));
    }
    if comp.len() == 2 && comp.iter().all(|a| *a == Atom::Ch('*', true)) {
        return Kind::Recursive;
    }
    let chars: Vec<(char, bool)> = comp
        .iter()
        .filter_map(|a| match a {
            Atom::Ch(c, g) => Some((*c, *g)),
            Atom::Unknown => None,
        })
        .collect();
    Kind::Glob(glob::parse(&chars))
}

/// Whether a file name's spelling on disk can differ from this spelling only
/// in ASCII case, which [`SecretGuard::is_denied`] folds anyway.
///
/// Printable ASCII (a space included), with none of `~`, `:` or the other
/// characters Windows reserves, and not ending in `.` or a space. That rules
/// out what a case-insensitive or legacy filesystem treats as another name for
/// the same file: a non-ASCII letter that folds to an ASCII one (`ſ` to `s` on
/// APFS), a short name (`CREDEN~1`), a trailing dot or space Windows drops, and
/// an alternate data stream (`name:stream`).
/// Whether the last component of `path` is itself a symbolic link.
fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|named| named.file_type().is_symlink())
}

#[cfg(test)]
pub(crate) fn is_stable_name_for_tests(name: &str) -> bool {
    is_stable_name(std::ffi::OsStr::new(name))
}

fn is_stable_name(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        !name.is_empty()
            && !name.ends_with(['.', ' '])
            && name.bytes().all(|b| {
                (b.is_ascii_graphic() || b == b' ')
                    && !matches!(
                        b,
                        b'~' | b':' | b'\\' | b'*' | b'?' | b'"' | b'<' | b'>' | b'|'
                    )
            })
    })
}

fn push_component(path: &mut PathBuf, name: &str) {
    match name {
        "" | "." => {}
        _ => path.push(name),
    }
}

/// A path with every component taken literally (glob characters as written,
/// holes as [`UNKNOWN`]).
fn literalize(start: &Path, comps: &[Template]) -> PathBuf {
    let mut path = start.to_path_buf();
    for comp in comps {
        push_component(&mut path, &expand::render(comp));
    }
    path
}

/// Fold `.` and `..` out of a path textually, the way `cd` does by default.
pub(crate) fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Canonicalize the deepest existing ancestor and re-append the rest, so a
/// path that does not exist yet still gets its parent's real identity.
pub(crate) fn canonical_prefix(path: &Path) -> PathBuf {
    let mut tail = Vec::new();
    let mut probe = path.to_path_buf();
    loop {
        if let Ok(canonical) = std::fs::canonicalize(&probe) {
            let mut out = canonical;
            for part in tail.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (probe.file_name().map(|n| n.to_os_string()), probe.parent()) {
            (Some(name), Some(parent)) if !parent.as_os_str().is_empty() => {
                tail.push(name);
                probe = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

fn shorten(text: &str) -> String {
    const MAX: usize = 200;
    let text = text.replace(UNKNOWN, "…");
    if text.chars().count() <= MAX {
        return text;
    }
    let mut out: String = text.chars().take(MAX).collect();
    out.push('…');
    out
}
