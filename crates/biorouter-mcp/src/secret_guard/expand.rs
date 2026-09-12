//! Word expansion for the secret guard: from lexed pieces to every string a
//! shell could produce for that word.
//!
//! Each output is a [`Template`]: characters tagged with whether the shell may
//! still treat them as glob syntax, and explicit [`Atom::Unknown`] holes where
//! the value cannot be known before the command runs (a command substitution,
//! a variable nobody set). Keeping the holes, instead of guessing, is what lets
//! [`super::resolve`] refuse `~/.ssh/$(ls ~/.ssh | head -1)` — it knows *where*
//! the unknown part is, even though it does not know what it is.
//!
//! Over-approximation is the rule. Where the shells disagree (bash globs the
//! value of an unquoted `$X`, zsh does not; zsh treats `(a|b)` in a word as
//! alternation, bash does not) every reading is produced, because an extra
//! reading can only add a refusal.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::lex::{ParamOp, Piece, Word, UNKNOWN};

/// Cap on the readings produced for one word (brace expansion, variables with
/// several possible values). Past it, the remaining holes collapse to
/// [`Atom::Unknown`], which is still checked, just less precisely.
pub(crate) const MAX_TEMPLATES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Atom {
    /// A character, and whether the shell may treat it as glob syntax.
    Ch(char, bool),
    /// Characters the guard cannot know before the command runs. May contain
    /// `/`, so a hole can span directories.
    Unknown,
}

pub(crate) type Template = Vec<Atom>;

/// The environment a command's words are expanded against.
///
/// Captured from the process for real scans and built by hand in tests, which
/// is what lets every test run against a throwaway HOME instead of the
/// operator's own `~/.aws`.
#[derive(Debug, Clone, Default)]
pub struct ShellEnv {
    vars: HashMap<String, String>,
    home: Option<PathBuf>,
}

impl ShellEnv {
    /// The daemon's own environment. The shell child inherits it (minus the
    /// daemon's private keys, which name no paths), so it is what `$HOME`,
    /// `$TMPDIR` and friends will expand to.
    pub fn from_process() -> Self {
        let vars: HashMap<String, String> = std::env::vars().collect();
        let home = vars
            .get("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                vars.get("USERPROFILE")
                    .filter(|h| !h.is_empty())
                    .map(PathBuf::from)
            })
            .or_else(|| etcetera::home_dir().ok());
        Self { vars, home }
    }

    /// An explicit environment. `HOME` among `vars` becomes the home directory.
    pub fn with_vars<I, K, V>(vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let vars: HashMap<String, String> = vars
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        let home = vars.get("HOME").map(PathBuf::from);
        Self { vars, home }
    }

    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str)
    }

    /// `~user`: the account database on Unix, nothing elsewhere.
    fn user_home(&self, user: &str) -> Option<PathBuf> {
        if self.home.is_some() && self.get("USER") == Some(user) {
            return self.home.clone();
        }
        user_home_from_system(user)
    }
}

#[cfg(unix)]
fn user_home_from_system(user: &str) -> Option<PathBuf> {
    use std::ffi::{CStr, CString};
    let name = CString::new(user).ok()?;
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    // SAFETY: every pointer refers to a live, correctly sized local, and the
    // returned `pw_dir` is copied out before `buf` goes out of scope.
    let rc = unsafe {
        libc::getpwnam_r(
            name.as_ptr(),
            &mut pwd,
            buf.as_mut_ptr(),
            buf.len(),
            &mut result,
        )
    };
    if rc != 0 || result.is_null() || pwd.pw_dir.is_null() {
        return None;
    }
    let dir = unsafe { CStr::from_ptr(pwd.pw_dir) };
    Some(PathBuf::from(dir.to_string_lossy().into_owned()))
}

#[cfg(not(unix))]
fn user_home_from_system(_user: &str) -> Option<PathBuf> {
    None
}

/// What the expansion of one word may consult besides the environment.
pub(crate) struct ExpandCtx<'a> {
    pub env: &'a ShellEnv,
    /// Values assigned earlier in the same command. Every assignment is kept,
    /// never replaced: the resolver does not follow control flow, so any of
    /// them may be the one in force.
    pub vars: &'a HashMap<String, Vec<Template>>,
    /// Possible values of `$PWD` (the directories the command may be in).
    pub pwd: &'a [Template],
}

/// One element of a word between brace expansion and the rest.
#[derive(Debug, Clone)]
enum Elem {
    Ch(char, bool),
    Hole(usize),
}

enum Hole<'p> {
    Param {
        name: &'p str,
        op: &'p ParamOp,
        quoted: bool,
    },
    Unknown,
}

/// Every reading of `word`. `assignment` enables the tilde expansion bash
/// performs after `=` and `:` in `NAME=value`.
pub(crate) fn expand_word(word: &Word, ctx: &ExpandCtx<'_>, assignment: bool) -> Vec<Template> {
    let mut holes: Vec<Hole<'_>> = Vec::new();
    let mut seq: Vec<Elem> = Vec::new();
    for piece in word {
        match piece {
            Piece::Lit { text, quoted } => {
                for c in text.chars() {
                    if c == UNKNOWN {
                        holes.push(Hole::Unknown);
                        seq.push(Elem::Hole(holes.len() - 1));
                    } else {
                        seq.push(Elem::Ch(c, *quoted));
                    }
                }
            }
            Piece::Param { name, op, quoted } => {
                holes.push(Hole::Param {
                    name,
                    op,
                    quoted: *quoted,
                });
                seq.push(Elem::Hole(holes.len() - 1));
            }
            Piece::Subst { .. } | Piece::Unknown => {
                holes.push(Hole::Unknown);
                seq.push(Elem::Hole(holes.len() - 1));
            }
        }
    }

    let mut alternatives = Vec::new();
    let mut budget = MAX_TEMPLATES;
    brace_expand(&seq, &mut alternatives, &mut budget, 0);
    let mut grouped = Vec::new();
    let mut budget = MAX_TEMPLATES;
    for alt in alternatives {
        group_expand(&alt, &mut grouped, &mut budget, 0);
    }

    let mut out: Vec<Template> = Vec::new();
    for alt in grouped {
        let alt = expand_tildes(alt, ctx, assignment);
        for template in fill_holes(&alt, &holes, ctx) {
            if !out.contains(&template) {
                out.push(template);
            }
            if out.len() >= MAX_TEMPLATES {
                return out;
            }
        }
    }
    out
}

/// Bash brace expansion: `{a,b}` and `{1..3}` in unquoted text.
fn brace_expand(seq: &[Elem], out: &mut Vec<Vec<Elem>>, budget: &mut usize, depth: u8) {
    if *budget == 0 {
        return;
    }
    if depth < 8 {
        for open in 0..seq.len() {
            if !matches!(seq[open], Elem::Ch('{', false)) {
                continue;
            }
            let Some((close, alternatives)) = brace_group(seq, open) else {
                continue;
            };
            for alt in alternatives {
                let mut combined = seq[..open].to_vec();
                combined.extend(alt);
                combined.extend_from_slice(&seq[close + 1..]);
                brace_expand(&combined, out, budget, depth + 1);
                if *budget == 0 {
                    return;
                }
            }
            return;
        }
    }
    out.push(seq.to_vec());
    *budget -= 1;
}

/// The alternatives of the brace group opening at `open`, and where it closes.
fn brace_group(seq: &[Elem], open: usize) -> Option<(usize, Vec<Vec<Elem>>)> {
    let mut depth = 0usize;
    let mut commas = Vec::new();
    let mut close = None;
    for (i, elem) in seq.iter().enumerate().skip(open) {
        match elem {
            Elem::Ch('{', false) => depth += 1,
            Elem::Ch('}', false) => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            Elem::Ch(',', false) if depth == 1 => commas.push(i),
            _ => {}
        }
    }
    let close = close?;
    if !commas.is_empty() {
        let mut alternatives = Vec::new();
        let mut start = open + 1;
        for comma in commas.into_iter().chain(std::iter::once(close)) {
            alternatives.push(seq[start..comma].to_vec());
            start = comma + 1;
        }
        return Some((close, alternatives));
    }
    // `{a..e}` / `{1..10..2}`: only when the whole body is plain text.
    let body: Option<String> = seq[open + 1..close]
        .iter()
        .map(|e| match e {
            Elem::Ch(c, false) => Some(*c),
            _ => None,
        })
        .collect();
    let body = body?;
    let parts: Vec<&str> = body.split("..").collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let step: i64 = parts
        .get(2)
        .map(|s| s.parse::<i64>().ok())
        .unwrap_or(Some(1))?
        .abs()
        .max(1);
    let items: Vec<String> = match (parts[0].parse::<i64>(), parts[1].parse::<i64>()) {
        (Ok(a), Ok(b)) => {
            let (lo, hi) = (a.min(b), a.max(b));
            if (hi - lo) / step > MAX_TEMPLATES as i64 {
                return None;
            }
            (0..)
                .map(|k| lo + k * step)
                .take_while(|v| *v <= hi)
                .map(|v| v.to_string())
                .collect()
        }
        _ => {
            let (a, b) = (single_char(parts[0])?, single_char(parts[1])?);
            let (lo, hi) = (a.min(b) as u32, a.max(b) as u32);
            (lo..=hi)
                .step_by(step as usize)
                .filter_map(char::from_u32)
                .map(String::from)
                .collect()
        }
    };
    let alternatives = items
        .into_iter()
        .map(|item| item.chars().map(|c| Elem::Ch(c, true)).collect())
        .collect();
    Some((close, alternatives))
}

fn single_char(s: &str) -> Option<char> {
    let mut chars = s.chars();
    let c = chars.next()?;
    (chars.next().is_none() && c.is_ascii_alphabetic()).then_some(c)
}

/// zsh groups and bash extglobs in a word: `(a|b)`, `@(a|b)`, `?(a)`, and the
/// ones that match open-ended runs (`*(…)`, `+(…)`, `!(…)`), which become `*`.
/// A group without `|` at the end of a word is a zsh glob qualifier (`*(.)`)
/// and is dropped.
fn group_expand(seq: &[Elem], out: &mut Vec<Vec<Elem>>, budget: &mut usize, depth: u8) {
    if *budget == 0 {
        return;
    }
    if depth < 8 {
        for open in 0..seq.len() {
            if !matches!(seq[open], Elem::Ch('(', false)) {
                continue;
            }
            let Some((close, alternatives)) = paren_group(seq, open) else {
                continue;
            };
            let prefix_char = match open.checked_sub(1).map(|i| &seq[i]) {
                Some(Elem::Ch(c @ ('@' | '!' | '*' | '+' | '?'), false)) => Some(*c),
                _ => None,
            };
            let head_end = if prefix_char.is_some() {
                open - 1
            } else {
                open
            };
            let head = &seq[..head_end];
            let tail = &seq[close + 1..];
            let mut readings: Vec<Vec<Elem>> = Vec::new();
            match prefix_char {
                Some('!' | '*' | '+') => readings.push(vec![Elem::Ch('*', false)]),
                Some('?') => {
                    readings.push(Vec::new());
                    readings.extend(alternatives);
                }
                Some(_) => readings.extend(alternatives),
                None if alternatives.len() == 1 && tail.is_empty() => readings.push(Vec::new()),
                None => readings.extend(alternatives),
            }
            for reading in readings {
                let mut combined = head.to_vec();
                combined.extend(reading);
                combined.extend_from_slice(tail);
                group_expand(&combined, out, budget, depth + 1);
                if *budget == 0 {
                    return;
                }
            }
            return;
        }
    }
    out.push(seq.to_vec());
    *budget -= 1;
}

fn paren_group(seq: &[Elem], open: usize) -> Option<(usize, Vec<Vec<Elem>>)> {
    let mut depth = 0usize;
    let mut bars = Vec::new();
    for (i, elem) in seq.iter().enumerate().skip(open) {
        match elem {
            Elem::Ch('(', false) => depth += 1,
            Elem::Ch(')', false) => {
                depth -= 1;
                if depth == 0 {
                    let mut alternatives = Vec::new();
                    let mut start = open + 1;
                    for bar in bars.iter().copied().chain(std::iter::once(i)) {
                        alternatives.push(seq[start..bar].to_vec());
                        start = bar + 1;
                    }
                    return Some((i, alternatives));
                }
            }
            Elem::Ch('|', false) if depth == 1 => bars.push(i),
            _ => {}
        }
    }
    None
}

/// Expand a leading `~`, `~user`, `~+`, `~-` (and, in an assignment, one after
/// every unquoted `:`).
fn expand_tildes(seq: Vec<Elem>, ctx: &ExpandCtx<'_>, assignment: bool) -> Vec<Elem> {
    let mut out = Vec::with_capacity(seq.len());
    let mut i = 0;
    let mut at_start = true;
    while i < seq.len() {
        if at_start && matches!(seq[i], Elem::Ch('~', false)) {
            let mut j = i + 1;
            let mut user = String::new();
            let mut plain = true;
            while j < seq.len() {
                match seq[j] {
                    Elem::Ch('/', _) => break,
                    Elem::Ch(':', false) if assignment => break,
                    Elem::Ch(c, false) => {
                        user.push(c);
                        j += 1;
                    }
                    _ => {
                        plain = false;
                        break;
                    }
                }
            }
            if plain {
                let home = match user.as_str() {
                    "" => ctx.env.home().map(Path::to_path_buf),
                    "+" => match ctx.pwd {
                        [only] => Some(PathBuf::from(render(only))),
                        _ => None,
                    },
                    "-" => None,
                    name => ctx.env.user_home(name),
                };
                match home {
                    Some(home) => {
                        out.extend(home.to_string_lossy().chars().map(|c| Elem::Ch(c, true)))
                    }
                    // Unknown home: keep a hole, never the literal `~`, so the
                    // path is still judged by its known suffix.
                    None => out.push(Elem::Hole(usize::MAX)),
                }
                i = j;
                at_start = false;
                continue;
            }
        }
        at_start = assignment && matches!(seq[i], Elem::Ch(':', false));
        out.push(seq[i].clone());
        i += 1;
    }
    out
}

fn fill_holes(seq: &[Elem], holes: &[Hole<'_>], ctx: &ExpandCtx<'_>) -> Vec<Template> {
    let mut results: Vec<Template> = vec![Vec::new()];
    for elem in seq {
        match elem {
            Elem::Ch(c, quoted) => {
                for r in &mut results {
                    r.push(Atom::Ch(*c, !*quoted));
                }
            }
            Elem::Hole(index) => {
                let values = match holes.get(*index) {
                    Some(hole) => hole_values(hole, ctx),
                    None => vec![vec![Atom::Unknown]],
                };
                let values = if results.len() * values.len() > MAX_TEMPLATES {
                    vec![vec![Atom::Unknown]]
                } else {
                    values
                };
                let mut next = Vec::with_capacity(results.len() * values.len());
                for r in &results {
                    for v in &values {
                        let mut combined = r.clone();
                        combined.extend(v.iter().cloned());
                        next.push(combined);
                    }
                }
                results = next;
            }
        }
    }
    results
}

fn hole_values(hole: &Hole<'_>, ctx: &ExpandCtx<'_>) -> Vec<Template> {
    let Hole::Param { name, op, quoted } = hole else {
        return vec![vec![Atom::Unknown]];
    };
    let glob_active = !*quoted;
    let as_template =
        |value: &str| -> Template { value.chars().map(|c| Atom::Ch(c, glob_active)).collect() };

    let mut values: Vec<Template> = ctx
        .vars
        .get(*name)
        .map(|assigned| {
            assigned
                .iter()
                .map(|t| {
                    t.iter()
                        .map(|a| match a {
                            Atom::Ch(c, g) => Atom::Ch(*c, *g && glob_active),
                            Atom::Unknown => Atom::Unknown,
                        })
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default();
    match *name {
        "HOME" => match ctx.env.home() {
            Some(home) => values.push(as_template(&home.to_string_lossy())),
            None => values.push(vec![Atom::Unknown]),
        },
        "PWD" => values.extend(ctx.pwd.iter().cloned()),
        name if name.len() == 1 && !name.chars().all(|c| c.is_ascii_alphabetic() || c == '_') => {
            values.push(vec![Atom::Unknown]);
        }
        name => match ctx.env.get(name) {
            Some(value) => values.push(as_template(value)),
            // Not set here — but a shell startup file (`~/.zshenv`) may set
            // it, so it is a hole rather than the empty string.
            None if values.is_empty() => values.push(vec![Atom::Unknown]),
            None => {}
        },
    }
    // A reading with no value at all would drop the whole word.
    if values.is_empty() {
        values.push(vec![Atom::Unknown]);
    }

    match op {
        ParamOp::Plain => {}
        ParamOp::OrDefault(word) => {
            values.extend(expand_word(word, ctx, false));
        }
        ParamOp::IfSet(word) => {
            values = expand_word(word, ctx, false);
            values.push(Vec::new());
        }
        ParamOp::Opaque => values = vec![vec![Atom::Unknown]],
    }
    values.dedup();
    if values.len() > MAX_TEMPLATES {
        values.truncate(MAX_TEMPLATES - 1);
        values.push(vec![Atom::Unknown]);
    }
    values
}

/// A template as text, holes as [`UNKNOWN`]. For nested scripts and messages.
pub(crate) fn render(template: &[Atom]) -> String {
    template
        .iter()
        .map(|a| match a {
            Atom::Ch(c, _) => *c,
            Atom::Unknown => UNKNOWN,
        })
        .collect()
}

/// A template from plain text: no holes, glob characters active.
pub(crate) fn literal(text: &str) -> Template {
    text.chars()
        .map(|c| {
            if c == UNKNOWN {
                Atom::Unknown
            } else {
                Atom::Ch(c, true)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::lex;
    use super::*;

    fn env() -> ShellEnv {
        ShellEnv::with_vars([("HOME", "/h"), ("USER", "me"), ("DIR", "/data")])
    }

    /// The POSIX reading, pinned — see `lex::lex_for`. These assertions are
    /// about the unix grammar (`cred\entials` is an escape, not a path
    /// separator) and they inverted on the Windows runner when the helper
    /// followed the host.
    fn expand(word: &str) -> Vec<String> {
        let tokens = lex::lex_for(word, false);
        let lex::Token::Word(w) = &tokens[0] else {
            panic!("not a word: {tokens:?}")
        };
        let vars = HashMap::new();
        let ctx = ExpandCtx {
            env: &env(),
            vars: &vars,
            pwd: &[],
        };
        expand_word(w, &ctx, false)
            .iter()
            .map(|t| render(t))
            .collect()
    }

    #[test]
    fn tilde_and_variables() {
        assert_eq!(expand("~/.aws/credentials"), vec!["/h/.aws/credentials"]);
        assert_eq!(expand("$HOME/.aws"), vec!["/h/.aws"]);
        assert_eq!(expand("${HOME}/.aws"), vec!["/h/.aws"]);
        assert_eq!(expand("\"$HOME\"/.aws"), vec!["/h/.aws"]);
        assert_eq!(expand("~me/x"), vec!["/h/x"]);
        // A quoted tilde is a literal character.
        assert_eq!(expand("'~'/x"), vec!["~/x"]);
        // Unset: a hole, not the empty string.
        assert_eq!(expand("$NOPE/.aws"), vec![format!("{UNKNOWN}/.aws")]);
        assert_eq!(
            expand("${NOPE:-~/.aws}/credentials"),
            vec![
                format!("{UNKNOWN}/credentials"),
                "/h/.aws/credentials".to_string()
            ]
        );
        assert_eq!(expand("$(pwd)/x"), vec![format!("{UNKNOWN}/x")]);
    }

    #[test]
    fn braces_and_groups() {
        assert_eq!(
            expand("~/.aws/{credentials,config}"),
            vec!["/h/.aws/credentials", "/h/.aws/config"]
        );
        assert_eq!(expand("f{1..3}"), vec!["f1", "f2", "f3"]);
        assert_eq!(expand("'{a,b}'"), vec!["{a,b}"]);
        assert_eq!(
            expand("~/.aws/(credentials|config)"),
            vec!["/h/.aws/credentials", "/h/.aws/config"]
        );
        assert_eq!(expand("~/.aws/*(.)"), vec!["/h/.aws/*"]);
        assert_eq!(expand("~/.aws/@(cred*)"), vec!["/h/.aws/cred*"]);
    }

    #[test]
    fn quote_splicing_and_ansi_c() {
        assert_eq!(
            expand("~/.aws/cred\"\"entials"),
            vec!["/h/.aws/credentials"]
        );
        assert_eq!(expand("~/.aws/cred\\entials"), vec!["/h/.aws/credentials"]);
        assert_eq!(
            expand("~/.aws/$'cred\\x65ntials'"),
            vec!["/h/.aws/credentials"]
        );
    }

    #[test]
    fn glob_activeness_follows_quoting() {
        let tokens = lex::lex_for("\"*\"x* $DIR", false);
        let lex::Token::Word(w) = &tokens[0] else {
            panic!()
        };
        let vars = HashMap::new();
        let ctx = ExpandCtx {
            env: &env(),
            vars: &vars,
            pwd: &[],
        };
        let t = &expand_word(w, &ctx, false)[0];
        assert_eq!(t[0], Atom::Ch('*', false));
        assert_eq!(t[2], Atom::Ch('*', true));
    }
}
