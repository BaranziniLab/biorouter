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
//! # What this cannot see
//!
//! A path computed at run time by something other than the shell — `python -c`
//! building it from pieces, base64, a script file written earlier — is beyond
//! any static check. That residue is why tool *output* is also scanned for
//! credential material (`biorouter::guardrails::secret_output`). Recursive
//! readers (`grep -r`, `tar`, `cp -r`, `find … -exec cat`) that name only an
//! ancestor directory are the other known gap, for the same reason.

use std::collections::{HashMap, VecDeque};
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
}

impl<'a> Resolver<'a> {
    pub(crate) fn new(guard: &'a SecretGuard, env: &'a ShellEnv) -> Self {
        Self {
            guard,
            env,
            canonicalize_left: MAX_CANONICALIZE,
            listings_left: MAX_LISTINGS,
            listings: HashMap::new(),
            secret_dirs: HashMap::new(),
            strict: true,
            dotglob: false,
            windows_lexing: cfg!(windows),
        }
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
            // budgets fail *open* when they run out — `denied_resolved` stops
            // canonicalising and answers "not denied", and `list` stops reading
            // directories — so a second pass running on the drain of the first
            // would be a weaker check than the first, which is precisely the
            // asymmetry the union exists to remove. The `listings` and
            // `secret_dirs` caches are deliberately *not* reset: they memoise
            // facts about the filesystem, which no grammar changes, so the
            // second pass reuses them for free.
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
        // Before any grammar runs, and for every entry point alike — a test
        // asking what another host would do must not silently lose the
        // dot-glob detection the production path performs.
        static DOT_QUALIFIER: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"\([^()|]*D[^()|]*\)").expect("static regex"));
        if DOT_QUALIFIER.is_match(command) {
            self.dotglob = true;
        }
        self.under_grammars(grammars, |me| {
            let mut state = State::new(bases);
            me.walk(command, &mut state)?;
            me.scan_code_in(command, &state)
        })
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
        })
    }

    // ---- the walk ---------------------------------------------------------

    fn walk(&mut self, script: &str, state: &mut State) -> Scan {
        if state.depth > MAX_NESTING {
            return Err(Finding {
                shown: shorten(script),
                path: None,
                reason: Reason::TooComplex,
            });
        }
        let mut words: Vec<Word> = Vec::new();
        let mut redirects: Vec<Redirect> = Vec::new();
        let mut awaiting_target = false;
        for token in lex::lex_for(script, self.windows_lexing) {
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
                Token::Redirect { op, body } => {
                    let heredoc = op == "<<" || op == "<<-";
                    redirects.push(Redirect { target: None, body });
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
        let mut i = 0;
        while let Some(word) = words.get(i) {
            match plain_text(word).as_deref() {
                Some(
                    "!" | "{" | "}" | "if" | "then" | "else" | "elif" | "fi" | "do" | "done"
                    | "while" | "until" | "esac" | "time" | "coproc" | "[[" | "]]" | "function",
                ) => i += 1,
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
        for word in args {
            self.check_word(word, state, &word_text(word))?;
        }
        for redirect in redirects {
            if let Some(target) = &redirect.target {
                self.check_word(target, state, &word_text(target))?;
            }
            if let Some(body) = &redirect.body {
                // A here-document is either data or a script (`bash <<EOF`,
                // `cat <<EOF | sh`, `ssh host <<EOF`). Read it as both.
                self.nested(body, state)?;
                self.scan_code_in(body, state)?;
            }
        }
        if args.is_empty() {
            return Ok(());
        }
        self.program_semantics(args, state)
    }

    fn program_semantics(&mut self, args: &[Word], state: &mut State) -> Scan {
        let (index, program) = self.program_of(args, state)?;
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

    /// The real program behind wrappers (`sudo`, `env X=1`, `command`, …).
    fn program_of(&mut self, args: &[Word], state: &mut State) -> Result<(usize, String), Finding> {
        let mut i = 0;
        while let Some(word) = args.get(i) {
            let name = program_name(word);
            if !matches!(
                name.as_str(),
                "builtin"
                    | "command"
                    | "exec"
                    | "nohup"
                    | "time"
                    | "nice"
                    | "sudo"
                    | "doas"
                    | "env"
                    | "caffeinate"
                    | "stdbuf"
                    | "timeout"
                    | "unbuffer"
                    | "noglob"
            ) {
                return Ok((i, name));
            }
            i += 1;
            while let Some(next) = args.get(i) {
                if let Some((n, value, append)) = split_assignment(next) {
                    self.assign(&n, &value, append, state)?;
                    i += 1;
                    continue;
                }
                let text = plain_text(next).unwrap_or_default();
                let is_duration =
                    name == "timeout" && text.starts_with(|c: char| c.is_ascii_digit());
                if text.starts_with('-') || is_duration {
                    i += 1;
                    continue;
                }
                break;
            }
        }
        Ok((args.len(), String::new()))
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
        if self.denied_resolved(&cur) {
            return Err(self.finding(shown, Some(cur), Reason::Pattern));
        }
        Ok(())
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

    /// Denied lexically, or once symlinks are resolved.
    fn denied_resolved(&mut self, path: &Path) -> bool {
        let lexical = lexical_normalize(path);
        if self.guard.is_denied(&lexical) {
            return true;
        }
        if self.canonicalize_left == 0 {
            return false;
        }
        self.canonicalize_left -= 1;
        let physical = canonical_prefix(path);
        physical != lexical && self.guard.is_denied(&physical)
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
        }
    }
}

// ---- helpers ---------------------------------------------------------------

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
