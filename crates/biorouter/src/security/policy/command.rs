//! Argv parsing + path canonicalization for the command policy engine.
//!
//! The old regex scanner matched patterns against the raw `Tool: … / args`
//! blob, so it was trivially evadable (`/usr/bin/env rm -rf /`, quote-splice,
//! `curl | bash` inside a subshell). `ParsedCommand` instead tokenizes argv,
//! unwraps `sudo`/`env`/`sh -c` wrappers, splits pipelines/subshells, and
//! canonicalizes path arguments against the session cwd, so a policy rule sees
//! the *resolved* binary and target rather than a string shape.
//!
//! BR-68 made this **dialect-aware**. A segment is tagged with the shell that
//! will actually interpret it (`Posix` / `PowerShell` / `Cmd`), because
//! applicability is (platform × dialect), not platform alone — `pwsh -c
//! "Remove-Item -Recurse -Force /"` on Linux is a real PowerShell command on a
//! POSIX filesystem. The POSIX path is defined to be **byte-for-byte** its
//! pre-BR-68 behaviour (the existing tests are the oracle); the new code only
//! runs for PowerShell/Cmd segments and for platform-targeted path
//! normalization.

use std::path::{Component, Path, PathBuf};

use super::target::{normalize_for, Blast, Dialect, EnvFacts, Platform, TargetPath};
use super::{cmd_shell, pwsh};

/// A path argument together with its blast classification, computed once at
/// parse time (against the real `EnvFacts`) so rule matching never has to
/// re-derive the environment.
#[derive(Debug, Clone)]
pub struct TargetHit {
    pub path: TargetPath,
    pub blast: Blast,
}

/// Wrappers whose leading occurrence is peeled away to reveal the real binary,
/// e.g. `sudo env FOO=1 rm …` -> `rm …`. Kept deliberately small. `runas` /
/// `start` are the Windows privilege / detach wrappers (BR-68).
const WRAPPER_BINARIES: &[&str] = &[
    "sudo", "env", "command", "exec", "nice", "nohup", "time", "stdbuf", "setsid", "doas", "xargs",
    "runas", "start",
];

/// Interpreters treated as "a shell" for `pipes_to_shell` / `sh -c` unwrapping.
/// BR-68 adds the Windows hosts (`powershell`, `pwsh`, `cmd`) and the PowerShell
/// *exec sinks* (`iex`, `Invoke-Expression`) so that `iwr … | iex` — the
/// canonical Windows RCE one-liner — sets `reads_shell` and is caught by the
/// existing `curl_pipe_shell` rule with no new rule at all.
const SHELL_BINARIES: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "fish",
    "dash",
    "csh",
    "tcsh",
    "ksh",
    "ash",
    "powershell",
    "pwsh",
    "cmd",
    "iex",
    "invoke-expression",
];

/// Depth cap so a maliciously nested `sh -c "sh -c …"` cannot recurse forever.
const MAX_UNWRAP_DEPTH: u8 = 4;

/// One pipeline stage / subshell command after tokenization + wrapper unwrap.
#[derive(Debug, Clone)]
pub struct Segment {
    /// Resolved `argv[0]`: basename for POSIX (`/usr/bin/rm` -> `rm`), canonical
    /// cmdlet for PowerShell (`ri` -> `Remove-Item`), lowercased binary for cmd.
    pub binary: String,
    /// Full token list (post wrapper-unwrap / alias+prefix normalization), for
    /// `arg_regex` / prefix matching.
    pub argv: Vec<String>,
    /// Canonicalized POSIX path arguments (relative resolved against cwd). Only
    /// populated for `Posix` segments — the existing baseline rules key off this,
    /// and it must stay byte-for-byte its pre-BR-68 value.
    pub paths: Vec<PathBuf>,
    /// The shell that will interpret this segment.
    pub dialect: Dialect,
    /// Path arguments normalized for the *target platform* (all dialects), plus
    /// their blast classification. What the BR-68 rules key off.
    pub targets: Vec<TargetHit>,
    /// The worst blast radius among `targets` (`Ordinary` if none).
    pub worst_blast: Blast,
    /// A PowerShell segment reached an execution sink with a non-literal command
    /// name (`&('Rem'+'ove-Item')`, `iex ($x)`), or an ambiguous parameter
    /// prefix / undecodable `-EncodedCommand` — the residue signal for the
    /// `ask`-tier obfuscation rule.
    pub obfuscated_exec: bool,
}

impl Segment {
    /// A normalized `binary arg1 arg2 …` line for `command_prefix` matching.
    pub fn command_line(&self) -> String {
        self.argv.join(" ")
    }

    /// Whether the normalized argv contains `flag` (case-insensitive). After
    /// PowerShell normalization `-rec` has already become `-Recurse`, so a floor
    /// matcher can ask for the canonical flag.
    pub fn has_arg(&self, flag: &str) -> bool {
        self.argv.iter().any(|a| a.eq_ignore_ascii_case(flag))
    }
}

/// A parsed command: the raw text plus every decomposed segment.
#[derive(Debug, Clone)]
pub struct ParsedCommand {
    pub raw: String,
    pub segments: Vec<Segment>,
    /// A stage pipes into (or is) an interpreter shell — the `curl | bash` shape.
    pub reads_shell: bool,
    /// The platform this command was parsed for.
    pub platform: Platform,
}

impl ParsedCommand {
    /// Parse for the running host. Backwards-compatible entry point: on our
    /// mac/Linux CI this selects the POSIX dialect and behaves exactly as before.
    pub fn parse(command: &str, cwd: &Path) -> ParsedCommand {
        let env = EnvFacts::host(&cwd.to_string_lossy());
        Self::parse_for(command, Platform::host(), &env)
    }

    /// Parse for an explicit target platform, using that platform's default
    /// shell dialect. This is what lets the Windows matrix run on a mac:
    /// `parse_for(cmd, Platform::Windows, &win_facts)`.
    pub fn parse_for(command: &str, platform: Platform, env: &EnvFacts) -> ParsedCommand {
        Self::parse_for_dialect(command, platform, platform.default_dialect(), env)
    }

    /// Parse with an explicit top-level dialect — used by the test matrix, whose
    /// rows state which shell the user is in (`(Windows, Cmd, "del /s /q C:\\")`),
    /// since a bare `del …` has no `cmd /c` wrapper to infer the dialect from.
    pub fn parse_for_dialect(
        command: &str,
        platform: Platform,
        top: Dialect,
        env: &EnvFacts,
    ) -> ParsedCommand {
        let mut segments = Vec::new();
        let mut reads_shell = false;
        parse_recursive(
            command,
            platform,
            top,
            env,
            0,
            &mut segments,
            &mut reads_shell,
        );

        if segments.is_empty() {
            segments.push(Segment {
                binary: String::new(),
                argv: command.split_whitespace().map(String::from).collect(),
                paths: Vec::new(),
                dialect: top,
                targets: Vec::new(),
                worst_blast: Blast::Ordinary,
                obfuscated_exec: false,
            });
        }

        ParsedCommand {
            raw: command.to_string(),
            segments,
            reads_shell,
            platform,
        }
    }
}

fn is_shell_binary(name: &str) -> bool {
    SHELL_BINARIES.contains(&name.to_ascii_lowercase().as_str())
}

/// Top-level operator following a segment (quote-aware split classifies these).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sep {
    Pipe,
    Other,
}

/// Split a command string into `(segment_text, following_sep)` respecting single
/// and double quotes. `&&`, `||`, `;`, `&`, newlines are command separators;
/// a lone `|` is a pipe (stays inside the same command).
///
/// Dialect-aware for `&`: in PowerShell a lone `&` is the **call operator**
/// (`&('Rem'+'ove-Item')`), not a command separator, so splitting on it would
/// destroy exactly the obfuscation shape we want to detect. Only `&&` (the PS7
/// chain operator) separates. In POSIX/Cmd a lone `&` still separates.
fn split_operators(text: &str, dialect: Dialect) -> Vec<(String, Sep)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_single {
            cur.push(c);
            if c == '\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            cur.push(c);
            if c == '"' {
                in_double = false;
            }
            i += 1;
            continue;
        }
        match c {
            '\'' => {
                in_single = true;
                cur.push(c);
            }
            '"' => {
                in_double = true;
                cur.push(c);
            }
            // #543: `>|` is the noclobber redirect, not a pipe; split there and
            // the write target became a command of its own, out of sight.
            '|' if dialect == Dialect::Posix && cur.ends_with('>') => cur.push(c),
            '|' => {
                if chars.get(i + 1) == Some(&'|') {
                    out.push((std::mem::take(&mut cur), Sep::Other));
                    i += 1;
                } else {
                    out.push((std::mem::take(&mut cur), Sep::Pipe));
                }
            }
            '&' => {
                let double = chars.get(i + 1) == Some(&'&');
                if dialect == Dialect::PowerShell && !double {
                    // Call operator — keep it in the token.
                    cur.push(c);
                } else {
                    out.push((std::mem::take(&mut cur), Sep::Other));
                    if double {
                        i += 1;
                    }
                }
            }
            ';' | '\n' | '\r' => {
                out.push((std::mem::take(&mut cur), Sep::Other));
            }
            _ => cur.push(c),
        }
        i += 1;
    }
    out.push((cur, Sep::Other));
    out
}

/// Group operator-split segments into commands, each command being its list of
/// pipeline stage strings.
fn split_into_commands(text: &str, dialect: Dialect) -> Vec<Vec<String>> {
    let mut commands: Vec<Vec<String>> = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    for (seg, sep) in split_operators(text, dialect) {
        if !seg.trim().is_empty() {
            cur.push(seg);
        }
        if sep == Sep::Other && !cur.is_empty() {
            commands.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        commands.push(cur);
    }
    commands
}

#[allow(clippy::too_many_arguments)]
fn parse_recursive(
    text: &str,
    platform: Platform,
    dialect: Dialect,
    env: &EnvFacts,
    depth: u8,
    segments: &mut Vec<Segment>,
    reads_shell: &mut bool,
) {
    for stages in split_into_commands(text, dialect) {
        if stages.len() >= 2 {
            for stage in stages.iter().skip(1) {
                if stage_binary_is_shell(stage, dialect) {
                    *reads_shell = true;
                }
            }
        }
        for stage in stages {
            parse_stage(&stage, platform, dialect, env, depth, segments, reads_shell);
        }
    }
}

/// Tokenize a stage for `dialect`, peel wrapper binaries, and return the
/// effective argv.
fn tokenize_stage(stage: &str, dialect: Dialect) -> Vec<String> {
    let tokens = match dialect {
        Dialect::Posix => shlex::split(stage)
            .unwrap_or_else(|| stage.split_whitespace().map(String::from).collect()),
        Dialect::PowerShell => pwsh::tokenize(stage),
        Dialect::Cmd => cmd_shell::tokenize(stage),
    };
    unwrap_wrappers(tokens)
}

fn stage_binary_is_shell(stage: &str, dialect: Dialect) -> bool {
    let tokens = tokenize_stage(stage, dialect);
    tokens
        .first()
        .map(|t| {
            let base = basename(t);
            // For PowerShell, an alias (`iex`) resolves to the sink name.
            is_shell_binary(&base) || is_shell_binary(&pwsh::resolve_alias(&base))
        })
        .unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
fn parse_stage(
    stage: &str,
    platform: Platform,
    dialect: Dialect,
    env: &EnvFacts,
    depth: u8,
    segments: &mut Vec<Segment>,
    reads_shell: &mut bool,
) {
    match dialect {
        Dialect::Posix => parse_posix_stage(stage, platform, env, depth, segments, reads_shell),
        Dialect::PowerShell => parse_pwsh_stage(stage, platform, env, depth, segments, reads_shell),
        Dialect::Cmd => parse_cmd_stage(stage, platform, env, depth, segments, reads_shell),
    }
}

/// The pre-BR-68 POSIX parse, verbatim except for tagging the segment with its
/// dialect + populating `targets` for the platform-aware rules.
fn parse_posix_stage(
    stage: &str,
    platform: Platform,
    env: &EnvFacts,
    depth: u8,
    segments: &mut Vec<Segment>,
    reads_shell: &mut bool,
) {
    let tokens = tokenize_stage(stage, Dialect::Posix);
    let Some(first) = tokens.first() else {
        return;
    };
    let binary = basename(first);
    let cwd = Path::new(&env.cwd);

    let path_tokens: Vec<&String> = tokens
        .iter()
        .skip(1)
        .filter(|t| is_posix_path_arg(t))
        .collect();
    let paths: Vec<PathBuf> = path_tokens.iter().map(|t| normalize_path(cwd, t)).collect();
    let (targets, worst_blast) =
        classify_targets(platform, env, path_tokens.iter().map(|s| s.as_str()));

    // Redirect targets (`> /etc/passwd`, `>> /dev/sda`) — not argv tokens, so
    // they are captured here and offered to the same rules (BR-68).
    let mut all_targets = targets;
    let mut worst = worst_blast;
    for rt in redirect_targets(stage) {
        let tp = normalize_for(platform, &rt, env);
        let b = super::target::classify(&tp, env);
        if blast_rank(b) > blast_rank(worst) {
            worst = b;
        }
        all_targets.push(TargetHit { path: tp, blast: b });
    }

    segments.push(Segment {
        binary: binary.clone(),
        argv: tokens.clone(),
        paths,
        dialect: Dialect::Posix,
        targets: all_targets,
        worst_blast: worst,
        obfuscated_exec: false,
    });

    maybe_recurse_interpreter(
        &binary,
        &tokens,
        platform,
        env,
        depth,
        segments,
        reads_shell,
    );
}

/// If `binary` is an interpreter, recurse into its inline script with the
/// *interpreter's* dialect — so `pwsh -c "Remove-Item …"` typed at a bash prompt
/// is still parsed as PowerShell (the segment the Windows rules key off), and
/// `cmd /c "powershell -Command …"` switches twice. This is the general form of
/// the `sh -c` unwrap the POSIX path always did.
#[allow(clippy::too_many_arguments)]
fn maybe_recurse_interpreter(
    binary: &str,
    tokens: &[String],
    platform: Platform,
    env: &EnvFacts,
    depth: u8,
    segments: &mut Vec<Segment>,
    reads_shell: &mut bool,
) {
    if depth >= MAX_UNWRAP_DEPTH {
        return;
    }
    if pwsh::is_powershell_host(binary) {
        if let Some(script) = extract_pwsh_host_script(tokens) {
            parse_recursive(
                &script,
                platform,
                Dialect::PowerShell,
                env,
                depth + 1,
                segments,
                reads_shell,
            );
        }
        return;
    }
    if cmd_shell::is_cmd_host(binary) {
        if let Some(script) = cmd_shell::extract_cmd_script(tokens) {
            if !script.trim().is_empty() {
                parse_recursive(
                    &script,
                    platform,
                    Dialect::Cmd,
                    env,
                    depth + 1,
                    segments,
                    reads_shell,
                );
            }
        }
        return;
    }
    if is_shell_binary(binary) {
        if let Some(script) = extract_dash_c_script(tokens) {
            parse_recursive(
                &script,
                platform,
                Dialect::Posix,
                env,
                depth + 1,
                segments,
                reads_shell,
            );
        }
    }
}

fn parse_pwsh_stage(
    stage: &str,
    platform: Platform,
    env: &EnvFacts,
    depth: u8,
    segments: &mut Vec<Segment>,
    reads_shell: &mut bool,
) {
    let tokens = pwsh::tokenize(stage);
    let unwrapped = unwrap_wrappers(tokens.clone());
    let Some(first) = unwrapped.first() else {
        return;
    };
    let binary = basename(first);

    // Record the segment (a `powershell -Command …` host is a marker so
    // `arg_regex` on the host still works), then recurse into any inline script
    // — `powershell -Command`, `-EncodedCommand`, or a nested `cmd /c`.
    push_pwsh_segment(&unwrapped, stage, platform, env, segments);
    maybe_recurse_interpreter(
        &binary,
        &unwrapped,
        platform,
        env,
        depth,
        segments,
        reads_shell,
    );
}

fn push_pwsh_segment(
    tokens: &[String],
    stage: &str,
    platform: Platform,
    env: &EnvFacts,
    segments: &mut Vec<Segment>,
) {
    let norm = pwsh::normalize(tokens);
    let (targets, worst) =
        classify_targets(platform, env, norm.positional.iter().map(|s| s.as_str()));
    let obfuscated = norm.ambiguous_param || pwsh::is_obfuscated_exec(stage, tokens);
    segments.push(Segment {
        binary: norm.cmdlet.clone(),
        argv: norm.argv.clone(),
        paths: Vec::new(),
        dialect: Dialect::PowerShell,
        targets,
        worst_blast: worst,
        obfuscated_exec: obfuscated,
    });
}

fn parse_cmd_stage(
    stage: &str,
    platform: Platform,
    env: &EnvFacts,
    depth: u8,
    segments: &mut Vec<Segment>,
    reads_shell: &mut bool,
) {
    let tokens = cmd_shell::tokenize(stage);
    let unwrapped = unwrap_wrappers(tokens.clone());
    let Some(first) = unwrapped.first() else {
        return;
    };
    let binary = basename(first).to_ascii_lowercase();

    // `cmd /c <script>` — unwrap and re-parse; `cmd /c powershell -Command …`
    // switches dialect a second time via the shared recursion helper.
    push_cmd_segment(&unwrapped, platform, env, segments);
    maybe_recurse_interpreter(
        &binary,
        &unwrapped,
        platform,
        env,
        depth,
        segments,
        reads_shell,
    );
}

fn push_cmd_segment(
    tokens: &[String],
    platform: Platform,
    env: &EnvFacts,
    segments: &mut Vec<Segment>,
) {
    let binary = basename(&tokens[0]).to_ascii_lowercase();
    let path_args: Vec<&str> = tokens
        .iter()
        .skip(1)
        .filter(|t| is_cmd_path_arg(t))
        .map(|s| s.as_str())
        .collect();
    let (targets, worst) = classify_targets(platform, env, path_args.into_iter());
    segments.push(Segment {
        binary,
        argv: tokens.to_vec(),
        paths: Vec::new(),
        dialect: Dialect::Cmd,
        targets,
        worst_blast: worst,
        obfuscated_exec: false,
    });
}

fn classify_targets<'a>(
    platform: Platform,
    env: &EnvFacts,
    tokens: impl Iterator<Item = &'a str>,
) -> (Vec<TargetHit>, Blast) {
    let mut targets = Vec::new();
    let mut worst = Blast::Ordinary;
    for t in tokens {
        let tp = normalize_for(platform, t, env);
        let b = super::target::classify(&tp, env);
        if blast_rank(b) > blast_rank(worst) {
            worst = b;
        }
        targets.push(TargetHit { path: tp, blast: b });
    }
    (targets, worst)
}

fn blast_rank(b: Blast) -> u8 {
    match b {
        Blast::Ordinary => 0,
        Blast::WildcardAtRoot => 1,
        Blast::HomeBare => 2,
        Blast::SystemDir => 3,
        Blast::Root => 4,
    }
}

/// Capture `>` / `>>` / `>|` (and `1>`, `2>`, `&>`, `>&file`) redirect targets
/// from a raw stage, each as the word the shell would write to.
///
/// #543: a target ends where the shell ends the word: unquoted whitespace, `;`,
/// `|`, `&`, `)`, `<` or `>`. It used to end only at whitespace or `;`, so the
/// `2>/dev/null)` of `x=$(ls 2>/dev/null)` was the path `/dev/null)`, not a
/// benign device, and the call was asked about as a write under `/dev`.
///
/// ⚠ A terminator inside quotes, after a backslash, or inside a `$(...)` /
/// `${...}` in the word does NOT end it: cutting `> "/tmp/x)/../../etc/passwd"`
/// at the `)` would leave `/tmp/x`, a temp path, and hide a write to
/// `/etc/passwd`. Quotes are removed from the word; a backslash is dropped only
/// before a character the shell would otherwise read as syntax, so a Windows
/// path keeps its separators.
pub(crate) fn redirect_targets(stage: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes: Vec<char> = stage.chars().collect();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '>' if !in_single && !in_double => {
                // A `>` that closes a contiguous `<word>` angle-bracket placeholder
                // (markdown/HTML prose like `<type>/`, `<slug>`, `</section>`) is a
                // tag close, not a shell redirect operator — no real shell writes via
                // `<word>`. Without this, documentation prose scanned as an embedded
                // command (sensitive_ops' execute_code body scan) mis-reads `<type>/`
                // as a redirect to `/`. Guarded by the sensitive_ops regression test
                // execute_code_markdown_angle_bracket_placeholder_is_not_flagged.
                if closes_angle_bracket_tag(&bytes, i) {
                    i += 1;
                    continue;
                }
                let mut j = i + 1;
                // `>>` appends; `>|`, the noclobber override, is still a write.
                if matches!(bytes.get(j), Some('>' | '|')) {
                    j += 1;
                }
                // `>&word`: a duplicated descriptor (`2>&1`, `>&-`) is no file,
                // any other word is one.
                let duplicates = bytes.get(j) == Some(&'&');
                if duplicates {
                    j += 1;
                }
                // `>(cmd)` is process substitution, not a file; the scan goes on
                // inside it, so a redirect within is still seen.
                if bytes.get(j) == Some(&'(') {
                    i = j;
                    continue;
                }
                while j < bytes.len() && bytes[j].is_whitespace() {
                    j += 1;
                }
                let (target, end) = redirect_word(&bytes, j);
                let descriptor =
                    duplicates && (target == "-" || target.chars().all(|ch| ch.is_ascii_digit()));
                if !target.is_empty() && !descriptor {
                    out.push(target);
                }
                i = end;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// The redirect target word starting at `start`, quotes removed, and the index
/// just past it. See [`redirect_targets`] for where a word ends.
fn redirect_word(chars: &[char], start: usize) -> (String, usize) {
    let mut word = String::new();
    let mut j = start;
    let mut quote: Option<char> = None;
    // Open `$(` / `${` (and their nested parens or braces) inside the word.
    let mut depth = 0usize;
    while j < chars.len() {
        let c = chars[j];
        if let Some(open) = quote {
            if c == open {
                quote = None;
            } else if open == '"' && c == '\\' && matches!(chars.get(j + 1), Some('"' | '\\')) {
                // `\"` stays inside the quotes; `\\` is kept as written and
                // cannot escape the quote that follows it.
                if chars[j + 1] == '"' {
                    word.push('"');
                } else {
                    word.push_str("\\\\");
                }
                j += 1;
            } else {
                word.push(c);
            }
            j += 1;
            continue;
        }
        if depth == 0 && (c.is_whitespace() || matches!(c, ';' | '|' | '&' | ')' | '<' | '>')) {
            break;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '\\' => match chars.get(j + 1) {
                Some(&next) => {
                    // Escaped syntax is part of the word; any other escape is
                    // kept as written (a Windows path's separator).
                    if !(next.is_whitespace()
                        || matches!(next, ';' | '|' | '&' | '(' | ')' | '<' | '>' | '\'' | '"'))
                    {
                        word.push('\\');
                    }
                    word.push(next);
                    j += 1;
                }
                None => word.push(c),
            },
            // `$'...'` / `$"..."`: the quotes open, the `$` is not in the word.
            '$' if matches!(chars.get(j + 1), Some('\'' | '"')) => {}
            '$' if matches!(chars.get(j + 1), Some('(' | '{')) => {
                word.push(c);
                word.push(chars[j + 1]);
                depth += 1;
                j += 1;
            }
            '(' | '{' if depth > 0 => {
                word.push(c);
                depth += 1;
            }
            ')' | '}' if depth > 0 => {
                word.push(c);
                depth -= 1;
            }
            _ => word.push(c),
        }
        j += 1;
    }
    (word, j)
}

/// True when the `>` at `gt_index` closes a contiguous `<word>` angle-bracket
/// placeholder: scanning left from just before it reaches a matching unquoted `<`
/// through a non-empty run of tag-word chars (alphanumeric or `-_./:`), with no
/// whitespace or other shell metacharacter in between. Contiguous only, so a real
/// redirect with any space around the operator (`echo hi > out`, `cmd 2> err`,
/// even `echo hi>fused.txt`) is never mistaken for a tag.
fn closes_angle_bracket_tag(bytes: &[char], gt_index: usize) -> bool {
    if gt_index == 0 {
        return false;
    }
    let mut k = gt_index;
    let mut saw_word = false;
    while k > 0 {
        let c = bytes[k - 1];
        if c == '<' {
            return saw_word;
        }
        if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':') {
            saw_word = true;
            k -= 1;
        } else {
            return false;
        }
    }
    false
}

/// Drop leading wrapper binaries (`sudo`, `env`, …) plus their flags and
/// `NAME=value` assignments, returning the argv of the real command.
fn unwrap_wrappers(tokens: Vec<String>) -> Vec<String> {
    let mut idx = 0;
    let mut guard = 0;
    while idx < tokens.len() && guard < WRAPPER_BINARIES.len() + 4 {
        guard += 1;
        let bin = basename(&tokens[idx]).to_ascii_lowercase();
        if !WRAPPER_BINARIES.contains(&bin.as_str()) {
            break;
        }
        idx += 1;
        // Skip this wrapper's own flags / assignments to reach the real argv[0].
        while idx < tokens.len() {
            let t = &tokens[idx];
            if t.starts_with('-') {
                // `sudo -u user` / `env -u NAME` consume a following value.
                idx += 1;
                if (t == "-u" || t == "--user") && idx < tokens.len() {
                    idx += 1;
                }
            } else if is_assignment(t) {
                idx += 1;
            } else {
                break;
            }
        }
    }
    if idx >= tokens.len() {
        tokens
    } else {
        tokens[idx..].to_vec()
    }
}

fn extract_dash_c_script(tokens: &[String]) -> Option<String> {
    for (i, t) in tokens.iter().enumerate() {
        // `-c`, `-lc`, `-xc`, … : a combined short-flag cluster ending in `c`.
        if t.starts_with('-') && !t.starts_with("--") && t.ends_with('c') && t.len() >= 2 {
            return tokens.get(i + 1).cloned();
        }
        if t == "--command" {
            return tokens.get(i + 1).cloned();
        }
    }
    None
}

/// `powershell -Command <script>` / `-EncodedCommand <b64>` → the inner script,
/// decoding base64 (UTF-16LE) for the encoded form.
fn extract_pwsh_host_script(tokens: &[String]) -> Option<String> {
    let mut i = 1;
    while i < tokens.len() {
        let t = &tokens[i];
        if let Some(sw) = pwsh::expand_host_switch(t) {
            match sw {
                "Command" => {
                    let rest = tokens[i + 1..].to_vec();
                    if !rest.is_empty() {
                        return Some(rest.join(" "));
                    }
                }
                "EncodedCommand" => {
                    if let Some(b64) = tokens.get(i + 1) {
                        // A payload that does not decode cleanly is left as a
                        // literal so the obfuscation residue rule can speak.
                        return Some(
                            pwsh::decode_encoded_command(b64).unwrap_or_else(|| b64.clone()),
                        );
                    }
                }
                _ => {}
            }
            // Host switches that take a value consume the next token.
            if matches!(sw, "ExecutionPolicy" | "WindowStyle" | "File") {
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    None
}

fn basename(token: &str) -> String {
    // Split on both separators so a Windows path in a POSIX-tokenized string
    // still yields the leaf. (`shlex` would have eaten the `\`, but a
    // whitespace-split fallback may not.)
    let leaf = token.rsplit(['/', '\\']).next().unwrap_or(token);
    let leaf = if leaf.is_empty() { token } else { leaf };
    leaf.to_string()
}

fn is_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether a token is a POSIX filesystem-path argument (so it gets canonicalized
/// and offered to `path_glob`). Flags and `NAME=value` are not. Byte-for-byte
/// the pre-BR-68 `is_path_arg`.
fn is_posix_path_arg(token: &str) -> bool {
    !token.is_empty() && !token.starts_with('-') && !is_assignment(token)
}

/// Whether a `cmd.exe` token is a path argument. `cmd` switches start with `/`
/// (`/f`, `/s`, `/q`), so — unlike POSIX — a `/`-leading switch is NOT a path;
/// but a real path target like `C:\` or `\\server\share` still is, and a
/// registry key (`HKLM\…`) is handled by its own rules, not as a path.
fn is_cmd_path_arg(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    if cmd_shell::as_switch(token).is_some() {
        return false;
    }
    if cmd_shell::is_registry_path(token) {
        return false;
    }
    !is_assignment(token)
}

/// Expand a leading `~` and lexically normalize a path against `cwd` WITHOUT
/// touching the filesystem — the pre-BR-68 POSIX normalizer, unchanged.
fn normalize_path(cwd: &Path, raw: &str) -> PathBuf {
    let expanded = expand_tilde(raw);
    let joined: PathBuf = if Path::new(&expanded).is_absolute() {
        PathBuf::from(&expanded)
    } else {
        cwd.join(&expanded)
    };

    let mut stack: Vec<Component> = Vec::new();
    for comp in joined.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(stack.last(), Some(Component::Normal(_))) {
                    stack.pop();
                }
            }
            other => stack.push(other),
        }
    }
    if stack.is_empty() {
        return PathBuf::from(".");
    }
    stack.iter().collect()
}

fn expand_tilde(raw: &str) -> String {
    if raw == "~" {
        return home_dir().unwrap_or_else(|| "~".to_string());
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return format!("{home}/{rest}");
        }
    }
    raw.to_string()
}

fn home_dir() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("USERPROFILE").ok().filter(|h| !h.is_empty()))
}

#[cfg(test)]
mod issue543_redirect_tests {
    use super::*;

    #[test]
    fn issue543_redirects_end_at_unquoted_shell_boundaries_and_accept_clobber() {
        for (command, expected) in [
            ("x=$(ls 2>/dev/null)", vec!["/dev/null"]),
            ("ls 2>/dev/null|head", vec!["/dev/null"]),
            ("ls 2>/dev/null&& echo ok", vec!["/dev/null"]),
            ("echo x >| /etc/passwd", vec!["/etc/passwd"]),
            ("echo x >|/etc/passwd", vec!["/etc/passwd"]),
            (
                "echo x >/tmp/ordinary|echo y >/etc/passwd",
                vec!["/tmp/ordinary", "/etc/passwd"],
            ),
        ] {
            assert_eq!(redirect_targets(command), expected, "{command}");
        }
    }

    #[test]
    fn issue543_redirect_capture_keeps_complete_quoted_and_escaped_words() {
        for command in [
            r#"echo x > "/tmp/name)|& with spaces""#,
            r#"echo x > '/tmp/name)|& with spaces'"#,
            r#"echo x > /tmp/name\)\|\&\ with\ spaces"#,
            r#"echo x > "/tmp/name)"'|& with spaces'"#,
        ] {
            let captured = redirect_targets(command);
            assert_eq!(captured.len(), 1, "{command}: {captured:?}");
            let expected = "/tmp/name)|& with spaces";
            let decoded = shlex::split(&captured[0])
                .filter(|words| words.len() == 1)
                .map(|mut words| words.remove(0));
            assert!(
                captured[0] == expected || decoded.as_deref() == Some(expected),
                "redirect target was truncated: {command}: {captured:?}"
            );
        }
        for command in ["echo x 2>&1", "echo x 1>&2", "knowledge/<type>/<slug>.md"] {
            assert!(redirect_targets(command).is_empty(), "{command}");
        }
    }

    #[test]
    fn issue543_policy_parser_preserves_protected_redirect_destinations() {
        let env =
            EnvFacts::for_platform(Platform::Linux, "/home/issue543/project", "/home/issue543");
        for command in [
            "echo x >/etc/passwd",
            "echo x >|/etc/passwd",
            r#"echo x > "/tmp/name)/../../etc/passwd""#,
            r#"echo x > "/tmp/name)"/../../etc/passwd"#,
            r#"echo x > /tmp/name\)/../../etc/passwd"#,
        ] {
            let parsed = ParsedCommand::parse_for(command, Platform::Linux, &env);
            assert!(
                parsed
                    .segments
                    .iter()
                    .flat_map(|segment| &segment.targets)
                    .any(|target| target.path.norm == "/etc/passwd"
                        && target.blast == Blast::SystemDir),
                "protected redirect vanished from policy targets: {command}: {:?}",
                parsed.segments
            );
        }
    }
}
