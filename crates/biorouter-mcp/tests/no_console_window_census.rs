//! **The console-window census**: every process BioRouter spawns on a Windows
//! user's machine is prepared with `CREATE_NO_WINDOW`, or has a row here saying
//! why it does not need to be.
//!
//! # Why this file exists
//!
//! `biorouterd` is started by the Electron main process with `windowsHide: true`,
//! so it owns no console. Every console-subsystem child it then spawns *without*
//! `CREATE_NO_WINDOW` makes Windows allocate a **new, visible console window**
//! for the lifetime of that child. Tool calls are short, so the user sees a black
//! window flash open and shut — once per shell call, per background-job poll, per
//! `git` probe, per Agent Drafter build.
//!
//! The flag had existed in the tree for a long time and was reached from six
//! spawn sites. Sixty-nine others never called it, including
//! `developer__shell` — the hottest path in the app. The reason was structural
//! rather than careless: the helper lived in `biorouter`, and `biorouter`
//! depends on `biorouter-mcp`, never the reverse, so the busiest spawn sites
//! *could not* call it. Moving the primitive down to `biorouter-sandbox` fixed
//! today's instances. This test is what stops tomorrow's: a new
//! `Command::new(...)` that forgets the flag fails here rather than shipping.
//!
//! # What it asserts
//!
//! It walks `crates/*/src/**.rs`, finds every `Command::new(`, and requires each
//! production site to be *covered* — the site itself, or the function containing
//! it, mentions one of the preparation helpers in [`COVERING_CALLS`]. Comments
//! and string literals are blanked before it looks, so neither a site nor a
//! covering call can be one that only a comment mentions.
//!
//! A site is `new` called on `Command` in any spelling the text shows:
//! `std::process::Command::new`, `tokio::process::Command::new`, a qualified
//! self type (`<std::process::Command>::new`, `<(Command)>::new`, `<Command as
//! Ext>::new`, after a keyword too: `return <Command>::new`), a turbofish (`Command::<>::new`), a raw identifier (`r#new`), a
//! `new` on any alias the tree gives it (`use … Command as Cmd`, `type Cmd =
//! Command;`, an alias of an alias, in any file), and `Self::new` or
//! `<Self>::new` inside an `impl` whose self type is `Command` or one of those
//! aliases. The file is read as one stream of tokens, so a call split across
//! lines is still one site, reported on the line its type is on.
//!
//! What it does NOT recognise, because the text does not say it (each would
//! need name resolution, which is what the socket gate borrows from clippy):
//! `Self::new` in a trait's DEFAULT method (the trait does not know its `Self`
//! is `Command`); `T::new()` for a generic `T` that is `Command` at the call
//! site; any call, alias or `impl` that only a macro's expansion spells out
//! (`$t::new` with the type passed in); and `Self` in an `impl` whose self type
//! is not a path ending in `Command` or an alias (`impl Ext for (Command)`).
//! And, as before, a `Command` built in one function and spawned in another.
//!
//! ⚠ **Why this stays a text scan** while the socket census beside it became a
//! compiler lint (`scripts/check-non-inheritable-sockets.sh`). The lint matches
//! a call, not what happens to its result: `disallowed_methods` on
//! `Command::new` would fire at every one of the hundred-odd spawn sites that
//! DO prepare their command, and each would need an allow attribute or a move
//! behind a helper. That is a change to every production spawn site, which is
//! more risk than this tripwire removes. What a text scan cannot see, it cannot
//! see; the paragraph above lists it.
//!
//! ⚠ **This is a coarse check and says so.** It proves a spawn site sits beside a
//! preparation call; it cannot prove the call applies to *that* command. It is a
//! tripwire for the omission that actually happened (a whole site with no
//! preparation anywhere near it), not a proof of correctness. A finer check would
//! need real dataflow, and a test nobody can maintain is worse than a coarse one
//! that fails loudly.
//!
//! ⚠ **It does not assume the walk worked.** A broken walk finds nothing and
//! reports success, which is how a gate passes forever while guarding nothing.
//! [`the_walk_actually_reads_the_tree`] pins a floor on the files and spawn sites
//! seen, and [`the_known_hot_paths_are_covered`] names specific files that must
//! be found *and* covered, so a walk that silently stops early fails.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Any of these appearing in the enclosing function means the child is prepared.
///
/// Several are choke points rather than the primitive itself — a site that calls
/// `prepare_agent_child_command` gets the flag through it, and naming the choke
/// point here is what lets one edit cover its many callers.
const COVERING_CALLS: &[&str] = &[
    "no_console_window",
    "configure_command_no_window",
    "prepare_agent_child_command",
    "prepare_agent_drafter_child",
    "configure_subscription_child",
    "child_process_client",
    "creation_flags",
];

// ⚠ Do NOT add a builder's own name here to "cover" its callers.
// `configure_shell_command` was listed once, on the reasoning that its callers
// inherit the preparation it does. But the name also matches the function's own
// *definition*, which is where the spawn is — so deleting the real
// `no_console_window` call from inside it left this census green. A negative
// control caught it (remove the call, watch the test still pass). An entry here
// must name a preparation helper the site *calls*, never the function the site
// lives in.

/// Spawn sites that genuinely need no flag, each with the reason.
///
/// A path here is matched as a suffix of the repo-relative path. Keep the reason
/// specific: "not Windows" is only true of a file that cannot compile on
/// Windows, which is a `cfg` fact, not a naming one.
///
/// ⚠ **An exemption is a hole, and holes widen quietly.** Four rows were removed
/// on 2026-09-22, and each had failed in a different way, which is the argument
/// for [`every_exemption_still_names_something_real`] below:
///
///  * two named `computercontroller/platform/{macos,linux}.rs`, which **no
///    longer exist**. A row whose path matches nothing exempts nothing today and
///    silently exempts whatever is created at that path tomorrow.
///  * one exempted `privacy/system_auth_polkit.rs` because "polkit is
///    Linux-only". That is a naming argument of exactly the kind the paragraph
///    above forbids: the file's own header says it "is compiled on every target,
///    not just Linux", deliberately, so a Windows build does reach it. The site
///    now sets the flag itself and needs no row.
///  * one exempted `agents/bug_report/issue.rs` because "`gh` runs only from the
///    maintainer bug-report flow, never on a user turn". `report_bug_tool()` is
///    offered to the model whenever `PlatformToolGates::bug_report` is on
///    (`agents/platform_tools.rs`), and the flow reaches `gh auth status` and
///    `gh issue create` — so it runs on an ordinary user turn, inside a daemon
///    that is started DETACHED and therefore hands its console-subsystem
///    children a brand-new, VISIBLE console. Both sites now set the flag.
const EXEMPT: &[(&str, &str)] = &[
    (
        "crates/biorouter-sandbox/src/shell_sandbox/linux.rs",
        "whole file is Linux-only (seccomp/landlock); never compiled for Windows",
    ),
    (
        "crates/biorouter/src/privacy/system_auth_macos.rs",
        "macOS authorization UI only",
    ),
    (
        "crates/biorouter/src/test_sandbox.rs",
        "test code: lib.rs declares it only as `#[cfg(test)] mod test_sandbox;`, so \
         it is never compiled into a shipped binary. Its spawn re-execs the test \
         binary itself for a test that must run in a process of its own.",
    ),
    (
        "crates/biorouter-cli/",
        "the CLI is a console application: its children inherit its console and \
         open no window of their own. Only the GUI daemon flashes.",
    ),
    (
        "crates/biorouter-bench/",
        "benchmark harness, not shipped in the desktop app",
    ),
    (
        "crates/biorouter-test/",
        "the integration-test harness crate; never shipped in the desktop app",
    ),
    (
        "crates/biorouter/src/providers/bedrock_namespace_tests.rs",
        "test scaffolding that re-execs the test binary itself, not a child of the running app",
    ),
    (
        "crates/biorouter-crew/src/broker.rs",
        "the file is included only by `#[cfg(unix)] mod broker` in biorouter-crew/src/lib.rs; its broker child therefore cannot compile on Windows",
    ),
    (
        "crates/biorouter-crew/src/remote.rs",
        "the file is included only by `#[cfg(unix)] pub mod remote` in biorouter-crew/src/lib.rs; its Linux remote helper child cannot compile on Windows",
    ),
];

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is <root>/crates/biorouter-mcp
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<crate>/ has two ancestors")
        .to_path_buf()
}

// The comment/literal stripper, the test-gate reader and the walk. This census
// used to carry its own copy of them, and the copy fell behind the one the
// (since retired) listener census kept: it skipped the newline after a `\` in a
// string (moving every later line of 256 files), misread `'\''`, and read a gate
// with no body (a field, a variant, a match arm, `mod tests;`, a `use`) as
// covering the next balanced block after it, so a spawn there was never checked.
#[path = "census_support/rust_source.rs"]
mod rust_source;

use rust_source::{
    is_test_only_cfg, region_containment_failures, rust_sources, strip_literals, test_module_lines,
    test_regions, RegionEnd, RegionKind,
};

// ---------------------------------------------------------------------------
// Reading code
// ---------------------------------------------------------------------------

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_ident(token: &str) -> bool {
    token
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && token.chars().all(is_ident_char)
}

/// The text with comments and literals blanked (lines kept where they were), and
/// raw identifiers written plainly: `Command::r#new` is `Command::new`. Raw
/// strings are already blank here, so every `r#` left starts an identifier.
fn code_of(text: &str) -> String {
    let stripped = strip_literals(text);
    let chars: Vec<char> = stripped.chars().collect();
    let mut out = String::with_capacity(stripped.len());
    let mut i = 0;
    while i < chars.len() {
        let raw_ident = chars[i] == 'r'
            && chars.get(i + 1) == Some(&'#')
            && chars.get(i + 2).is_some_and(|&c| is_ident_char(c))
            && (i == 0 || !is_ident_char(chars[i - 1]));
        if raw_ident {
            i += 2;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Identifiers, `::`, and every other non-blank character on its own.
fn tokens(code: &str) -> Vec<&str> {
    tokens_with_lines(code)
        .into_iter()
        .map(|(t, _)| t)
        .collect()
}

/// [`tokens`], each with the 1-based line it is on. A site is found in the
/// whole file's tokens rather than line by line, so `Command` and `::new` on
/// two lines are still one call.
fn tokens_with_lines(code: &str) -> Vec<(&str, usize)> {
    let mut out = Vec::new();
    let mut line = 1;
    let mut iter = code.char_indices().peekable();
    while let Some((start, c)) = iter.next() {
        if c == '\n' {
            line += 1;
            continue;
        }
        if c.is_whitespace() {
            continue;
        }
        let mut end = start + c.len_utf8();
        if is_ident_char(c) {
            while let Some(&(at, next)) = iter.peek() {
                if !is_ident_char(next) {
                    break;
                }
                end = at + next.len_utf8();
                iter.next();
            }
        } else if c == ':' && iter.peek().is_some_and(|&(_, next)| next == ':') {
            iter.next();
            end += 1;
        }
        // `get`, not `[..]`: clippy::string_slice. Both ends are char boundaries.
        if let Some(token) = code.get(start..end) {
            out.push((token, line));
        }
    }
    out
}

/// Whether `name` spawns: `Command` (any type named `…Command`, as the old
/// substring rule counted `TokioCommand::new`), or one of `aliases`.
fn names_command(name: &str, aliases: &BTreeSet<String>) -> bool {
    name.ends_with("Command") || aliases.contains(name)
}

/// Every other name `codes` give `Command`: `use … Command as X` (in a group
/// too), `type X = …::Command;`, and an alias of an alias, to a fixpoint. The
/// names are collected across the whole tree, because a `pub use … as` in one
/// file is spelled bare in another, and a same-named type elsewhere costs an
/// over-report at worst.
fn command_aliases(codes: &[&str]) -> BTreeSet<String> {
    let mut aliases = BTreeSet::new();
    loop {
        let before = aliases.len();
        // Tokenized afresh each round rather than kept: the whole tree's tokens
        // at once would be tens of megabytes, and a round is quick.
        for code in codes {
            let toks = tokens(code);
            for (i, &tok) in toks.iter().enumerate() {
                // `Command as Cmd`
                if tok == "as" && i > 0 {
                    if let Some(&alias) = toks.get(i + 1) {
                        if names_command(toks[i - 1], &aliases) && is_ident(alias) && alias != "_" {
                            aliases.insert(alias.to_string());
                        }
                    }
                }
                // `type Cmd = std::process::Command;`, `type Cmd<T> = …;`
                if tok == "type" {
                    let Some(&alias) = toks.get(i + 1) else {
                        continue;
                    };
                    let rest = toks.get(i + 2..).unwrap_or_default();
                    let Some(eq) = rest.iter().position(|t| *t == "=" || *t == ";") else {
                        continue;
                    };
                    if rest[eq] != "=" || !is_ident(alias) {
                        continue;
                    }
                    let rhs = rest.get(eq + 1..).unwrap_or_default();
                    let rhs = &rhs[..rhs.iter().position(|t| *t == ";").unwrap_or(rhs.len())];
                    // The last path segment outside any generic arguments.
                    let mut depth = 0i32;
                    let mut last = None;
                    for &t in rhs {
                        match t {
                            "<" => depth += 1,
                            ">" => depth -= 1,
                            t if depth == 0 && is_ident(t) => last = Some(t),
                            _ => {}
                        }
                    }
                    if last.is_some_and(|l| names_command(l, &aliases)) {
                        aliases.insert(alias.to_string());
                    }
                }
            }
        }
        if aliases.len() == before {
            return aliases;
        }
    }
}

/// The lines on which `code` calls `new` on `Command`, an alias of it, or
/// `Self` in an impl of it: `Command::new`, `Cmd::new`, `<Cmd>::new`,
/// `<(Command)>::new`, `<Command as Ext>::new`, `Command::<>::new`, and
/// `Self::new` or `<Self>::new` in `impl … for std::process::Command`. The
/// line is the type's, which for a call split across lines is its first.
fn spawn_site_lines(code: &str, aliases: &BTreeSet<String>) -> BTreeSet<usize> {
    let toks = tokens_with_lines(code);
    let words: Vec<&str> = toks.iter().map(|&(t, _)| t).collect();
    let impls = impl_bodies(&words, aliases);
    let mut lines = BTreeSet::new();
    for (i, &(tok, line)) in toks.iter().enumerate() {
        let names_it = if tok == "Self" {
            // The innermost impl around this `Self` is the one it names.
            impls
                .iter()
                .filter(|body| body.open < i && i < body.close)
                .max_by_key(|body| body.open)
                .is_some_and(|body| body.of_command)
        } else {
            names_command(tok, aliases)
        };
        if names_it && calls_new_after(&words, i).is_some() {
            lines.insert(line);
        }
    }
    lines
}

/// Some when the type named at `t[i]` goes on to `::new`, through the
/// shapes a path can take between the two.
fn calls_new_after(t: &[&str], i: usize) -> Option<()> {
    let mut j = i + 1;
    // Still inside a qualified self type: `<(Command)>`, `<Command as Ext>`.
    while t.get(j) == Some(&")") {
        j += 1;
    }
    if t.get(j) == Some(&"as") {
        j += 1;
        loop {
            match t.get(j) {
                Some(&"::") => j += 1,
                Some(&"<") => j = past_angles(t, j)?,
                Some(&tok) if is_ident(tok) => j += 1,
                _ => break,
            }
        }
        // `as` inside anything but a qualified self (`use … as X;`) is not a
        // call at all.
        if t.get(j) != Some(&">") {
            return None;
        }
    }
    if t.get(j) == Some(&">") && !is_arrow(t, j) {
        // Only the `>` of a qualified self type: `= <Command>::new`. The `>` of
        // generic arguments (`Vec::<Command>::new`, `<Vec<Command>>::new`)
        // closes a `<` that follows a name or a `::`, and that `new` is not
        // `Command`'s. A keyword is not a name: `return <Command>::new(p)`,
        // `if <Command>::new(p)…` and `&mut <Command>::new(p)` are each a
        // qualified self type.
        let open = matching_open_angle(t, j)?;
        if open
            .checked_sub(1)
            .is_some_and(|k| (is_ident(t[k]) && !KEYWORDS.contains(&t[k])) || t[k] == "::")
        {
            return None;
        }
        j += 1;
    }
    // A turbofish, empty or not: `Command::<>::new`.
    if t.get(j) == Some(&"::") && t.get(j + 1) == Some(&"<") {
        j = past_angles(t, j + 1)?;
    }
    (t.get(j) == Some(&"::") && t.get(j + 1) == Some(&"new")).then_some(())
}

/// Rust's keywords, strict and reserved, less the four that are path segments
/// (`self`, `Self`, `super`, `crate`). None of them is a path a `<` could give
/// generic arguments to.
const KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "do", "dyn",
    "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl", "in", "let",
    "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref", "return",
    "static", "struct", "trait", "true", "try", "type", "typeof", "unsafe", "unsized", "use",
    "virtual", "where", "while", "yield",
];

/// Whether the `>` at `t[j]` is the second half of `->`.
fn is_arrow(t: &[&str], j: usize) -> bool {
    j > 0 && t[j - 1] == "-"
}

/// The index just past the `>` matching the `<` at `t[j]`.
fn past_angles(t: &[&str], j: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (k, &tok) in t.iter().enumerate().skip(j) {
        match tok {
            "<" => depth += 1,
            ">" if !is_arrow(t, k) => {
                depth -= 1;
                if depth == 0 {
                    return Some(k + 1);
                }
            }
            ";" | "{" | "}" => return None,
            _ => {}
        }
    }
    None
}

/// The index of the `<` that the `>` at `t[j]` closes.
fn matching_open_angle(t: &[&str], j: usize) -> Option<usize> {
    let mut depth = 0i32;
    for k in (0..j).rev() {
        match t[k] {
            ">" if !is_arrow(t, k) => depth += 1,
            "<" => {
                if depth == 0 {
                    return Some(k);
                }
                depth -= 1;
            }
            ";" | "{" | "}" => return None,
            _ => {}
        }
    }
    None
}

/// One `impl` block: the token indices of its braces, and whether its self
/// type is `Command` or an alias of it.
struct ImplBody {
    open: usize,
    close: usize,
    of_command: bool,
}

/// Every `impl` item in the tokens. An `impl` in type position (`-> impl
/// Trait`, `x: impl Trait`) is not an item, and is told apart by what comes
/// before it: an item starts a file or follows a `;`, a brace, an attribute's
/// `]`, `unsafe` or `default`.
fn impl_bodies(t: &[&str], aliases: &BTreeSet<String>) -> Vec<ImplBody> {
    let mut out = Vec::new();
    for (i, &tok) in t.iter().enumerate() {
        if tok != "impl" {
            continue;
        }
        let item = i == 0 || matches!(t[i - 1], ";" | "{" | "}" | "]" | "unsafe" | "default");
        if !item {
            continue;
        }
        let mut j = i + 1;
        if t.get(j) == Some(&"<") {
            let Some(after) = past_angles(t, j) else {
                continue;
            };
            j = after;
        }
        // The header runs to the body's `{`; the self type is what follows a
        // top-level `for`, or the whole header when there is none, up to a
        // `where`. Only a `for` BEFORE the `where` is the impl's: one after it
        // is a higher-ranked bound (`where F: for<'a> Fn(&'a str)`,
        // `where for<'a> &'a T: Sized`), and reading it as the impl's put the
        // self type's start after its end.
        let mut depth = 0i32;
        let mut self_start = j;
        let mut self_end = None;
        let mut open = None;
        for (k, &tok) in t.iter().enumerate().skip(j) {
            match tok {
                "<" | "(" | "[" => depth += 1,
                ">" if !is_arrow(t, k) => depth -= 1,
                ")" | "]" => depth -= 1,
                "for" if depth == 0 && self_end.is_none() => self_start = k + 1,
                "where" if depth == 0 => self_end = self_end.or(Some(k)),
                "{" if depth == 0 => {
                    open = Some(k);
                    break;
                }
                ";" => break,
                _ => {}
            }
        }
        let Some(open) = open else {
            continue;
        };
        // Never a panic: a header this reader cannot take apart fails CLOSED,
        // as an impl of `Command`, so every `Self::new` in it is a spawn site
        // that must be covered, and the reason is printed beside the failure.
        let Some(self_type) = t.get(self_start..self_end.unwrap_or(open)) else {
            eprintln!(
                "census: could not read the self type of `{}`; every `Self::new` in it \
                 is treated as a spawn site",
                t.get(i..open).unwrap_or_default().join(" ")
            );
            out.extend(body_close(t, open).map(|close| ImplBody {
                open,
                close,
                of_command: true,
            }));
            continue;
        };
        // The last path segment outside any generic arguments.
        let mut depth = 0i32;
        let mut last = None;
        for &tok in self_type {
            match tok {
                "<" => depth += 1,
                ">" => depth -= 1,
                tok if depth == 0 && is_ident(tok) => last = Some(tok),
                _ => {}
            }
        }
        let of_command = last.is_some_and(|l| l == "Command" || aliases.contains(l));
        out.extend(body_close(t, open).map(|close| ImplBody {
            open,
            close,
            of_command,
        }));
    }
    out
}

/// The index of the `}` closing the `{` at `t[open]`.
fn body_close(t: &[&str], open: usize) -> Option<usize> {
    let mut braces = 0i32;
    t.iter().enumerate().skip(open).find_map(|(k, &tok)| {
        match tok {
            "{" => braces += 1,
            "}" => braces -= 1,
            _ => {}
        }
        (braces == 0).then_some(k)
    })
}

// ---------------------------------------------------------------------------
// Judging a site
// ---------------------------------------------------------------------------

/// Whether a `cfg` gate that excludes Windows governs the site on `line`
/// (1-based): the statement's own attribute, or one on the function it is in.
///
/// This is the honest reason most remaining sites need no flag: a `ps`, an
/// `xdotool` or an `osascript` branch is never compiled for Windows, so it can
/// no more flash a console than it can run. Reading the gate beats listing the
/// lines, because the gate moves with the code.
///
/// `raw` is the file as written, which the gate is read from (a `cfg` names its
/// target in a string, which `code` has blanked); `code` is the same lines
/// stripped, which says where the comments and the braces are.
fn gated_away_from_windows(raw: &[&str], code: &[&str], line: usize) -> bool {
    const NOT_WINDOWS: &[&str] = &[
        "#[cfg(unix)]",
        "#[cfg(not(windows))]",
        "#[cfg(target_os = \"linux\")]",
        "#[cfg(target_os = \"macos\")]",
        "#[cfg(not(target_os = \"windows\"))]",
        "#[cfg(any(target_os = \"linux\", target_os = \"macos\"))]",
    ];
    let is_gate = |l: &str| NOT_WINDOWS.iter().any(|g| l.trim_start().starts_with(g));
    let idx = line - 1;

    // ⚠ Only a gate that governs THIS statement counts. A first draft scanned
    // back ten lines and exempted a spawn because an unrelated
    // `#[cfg(not(windows))] let node = …` sat above it — the gate belonged to a
    // different binding, and the spawn ran on Windows regardless. So: the
    // nearest line above that holds code, and nothing further.
    //
    // ⚠ And only a line that is NOTHING BUT attributes. `#[cfg(unix)] let _ =
    // ();` on the line above a spawn gates that `let`, not the spawn, which
    // runs on Windows regardless; reading only how the line starts exempted it.
    if let Some(above) = (0..idx).rev().find(|&k| !code[k].trim().is_empty()) {
        if is_gate(raw[above]) && only_attributes(code[above]) {
            return true;
        }
    }

    // ⚠ The attributes of the function the site is IN, found by its braces. The
    // previous search took the nearest UNINDENTED `fn` above the line, so a
    // spawn in an impl method was judged by whatever free function sat above
    // the impl, and a `#[cfg(unix)] fn` there exempted a method that runs on
    // Windows.
    let Some(function) = enclosing_fn(code, idx) else {
        return false;
    };
    for k in (0..function).rev() {
        let attribute = code[k].trim();
        if attribute.is_empty() {
            continue; // a comment, a doc comment, a blank line
        }
        if !only_attributes(attribute) {
            break; // the function's attributes end here
        }
        if is_gate(raw[k]) {
            return true;
        }
    }
    false
}

/// Whether this line of code holds one or more outer attributes and nothing
/// else: `#[cfg(unix)]`, `#[cfg(unix)] #[allow(x)]`, but not `#[cfg(unix)]
/// let _ = ();` or `#[cfg(unix)] use x;`. Read from the stripped line, where a
/// `]` inside a string is already blank.
fn only_attributes(code_line: &str) -> bool {
    let mut rest = code_line.trim();
    if !rest.starts_with("#[") {
        return false;
    }
    while let Some(attribute) = rest.strip_prefix("#[") {
        let mut depth = 1i32;
        let mut end = None;
        for (at, c) in attribute.char_indices() {
            match c {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(at);
                        break;
                    }
                }
                _ => {}
            }
        }
        // An attribute that does not close on this line is not one this can
        // vouch for.
        let Some(end) = end else {
            return false;
        };
        // `]` is one byte, so `end + 1` is a boundary.
        rest = attribute.get(end + 1..).unwrap_or_default().trim_start();
    }
    rest.is_empty()
}

/// The 0-based line of the innermost function whose body holds line `idx`.
fn enclosing_fn(code: &[&str], idx: usize) -> Option<usize> {
    (0..=idx)
        .rev()
        .filter(|&k| is_fn_start(code[k]))
        .find(|&k| fn_body_end(code, k).is_some_and(|end| end >= idx))
}

/// The 0-based last line of the body of the function whose signature starts
/// on `start`; `None` for one with no body (`fn f();` in a trait).
fn fn_body_end(code: &[&str], start: usize) -> Option<usize> {
    let mut braces = 0i32;
    let mut nest = 0i32;
    for (k, line) in code.iter().enumerate().skip(start) {
        for c in line.chars() {
            match c {
                '{' => braces += 1,
                '}' => {
                    braces -= 1;
                    if braces == 0 {
                        return Some(k);
                    }
                }
                '(' | '[' => nest += 1,
                ')' | ']' => nest -= 1,
                // `;` in the signature ends a declaration; `[u8; 4]` is nested.
                ';' if braces == 0 && nest == 0 => return None,
                _ => {}
            }
        }
    }
    None
}

/// The code of the item containing `line`, approximated as the span back to the
/// previous `fn` and forward to the next one. Coarse on purpose — see the module
/// header. It is read from the STRIPPED lines: a comment that mentions
/// `no_console_window` beside a spawn prepares nothing.
fn enclosing_item(code: &[&str], line: usize) -> String {
    let idx = line - 1;
    let start = (0..idx).rev().find(|&n| is_fn_start(code[n])).unwrap_or(0);
    let end = (idx + 1..code.len())
        .find(|&n| is_fn_start(code[n]))
        .unwrap_or(code.len());
    code[start..end].join("\n")
}

/// Does this line begin a function?
///
/// ⚠ Must cover **every** visibility spelling, not the common ones. A first
/// draft listed `fn` / `pub fn` / `async fn` literally and so did not recognise
/// `pub(crate) fn`. The span for a spawn therefore ran past the end of its own
/// function into the next one, found *that* function's `no_console_window` call,
/// and pronounced the site covered. The negative control — delete the real call
/// and watch the census stay green — is what exposed it.
fn is_fn_start(line: &str) -> bool {
    let mut rest = line.trim_start();
    // `pub`, `pub(crate)`, `pub(super)`, `pub(in path)` …
    if let Some(after_pub) = rest.strip_prefix("pub") {
        rest = match after_pub.strip_prefix('(') {
            Some(scoped) => match scoped.find(')') {
                // `find` returns a character boundary and `)` is one byte,
                // so `close + 1` is a boundary too. `get` enforces it.
                Some(close) => match scoped.get(close + 1..) {
                    Some(after) => after.trim_start(),
                    None => return false,
                },
                None => return false,
            },
            None => after_pub.trim_start(),
        };
    }
    loop {
        let before = rest;
        for prefix in ["default ", "const ", "async ", "unsafe ", "extern "] {
            if let Some(stripped) = rest.strip_prefix(prefix) {
                rest = stripped.trim_start();
            }
        }
        // `extern "C" fn`, when the ABI string has not been blanked
        if rest.starts_with('"') {
            // `"` is one byte, so index 1 and `close + 2` are both boundaries.
            if let Some(close) = rest.get(1..).and_then(|tail| tail.find('"')) {
                match rest.get(close + 2..) {
                    Some(after) => rest = after.trim_start(),
                    None => break,
                }
            }
        }
        if rest == before {
            break;
        }
    }
    rest.starts_with("fn ")
}

#[derive(Debug)]
struct Site {
    file: String,
    line: usize,
    code: String,
}

/// The uncovered production spawn sites in one file, and how many spawn sites
/// it has in all. `rel` is the repo-relative path, which `EXEMPT` is matched
/// against; `tree_aliases` are the names other files give `Command`, added to
/// the ones this file gives it.
fn uncovered_in(rel: &str, text: &str, tree_aliases: &BTreeSet<String>) -> (Vec<Site>, usize) {
    let code = code_of(text);
    let mut aliases = command_aliases(&[code.as_str()]);
    aliases.extend(tree_aliases.iter().cloned());

    let mut uncovered = Vec::new();
    let mut total_sites = 0usize;
    let in_test = test_module_lines(&code);
    let raw: Vec<&str> = text.lines().collect();
    let lines: Vec<&str> = code.lines().collect();
    assert_eq!(
        raw.len(),
        lines.len(),
        "{rel}: reading the code moved its lines, so every site would be misplaced"
    );

    for line in spawn_site_lines(&code, &aliases) {
        let idx = line - 1;
        total_sites += 1;
        if in_test.contains(&line) {
            continue;
        }
        if EXEMPT.iter().any(|(p, _)| rel.contains(p)) {
            continue;
        }
        if gated_away_from_windows(&raw, &lines, line) {
            continue;
        }
        let item = enclosing_item(&lines, line);
        if COVERING_CALLS.iter().any(|c| item.contains(c)) {
            continue;
        }
        uncovered.push(Site {
            file: rel.to_string(),
            line,
            code: raw[idx].trim_start().chars().take(80).collect(),
        });
    }
    (uncovered, total_sites)
}

/// One `.rs` file under `crates/*/src`, read once.
struct Source {
    /// Repo-relative, with `/` separators.
    rel: String,
    text: String,
    /// `code_of(text)`.
    code: String,
}

/// Every `.rs` file under `crates/*/src`, and the names the tree gives
/// `Command`. Read once per test binary; every test here reads the same tree.
fn tree() -> &'static (Vec<Source>, BTreeSet<String>) {
    static TREE: OnceLock<(Vec<Source>, BTreeSet<String>)> = OnceLock::new();
    TREE.get_or_init(|| {
        let root = repo_root();
        let sources: Vec<Source> = rust_sources(&root)
            .iter()
            .filter_map(|path| {
                let text = std::fs::read_to_string(path).ok()?;
                let rel = path
                    .strip_prefix(&root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .replace('\\', "/");
                let code = code_of(&text);
                Some(Source { rel, text, code })
            })
            .collect();
        let codes: Vec<&str> = sources.iter().map(|s| s.code.as_str()).collect();
        let aliases = command_aliases(&codes);
        (sources, aliases)
    })
}

/// The files the census reads: every source with at least one spawn site.
fn spawning_files() -> Vec<&'static Source> {
    let (sources, aliases) = tree();
    sources
        .iter()
        .filter(|s| !spawn_site_lines(&s.code, aliases).is_empty())
        .collect()
}

fn uncovered_sites() -> (Vec<Site>, usize, usize) {
    let (sources, aliases) = tree();
    let mut uncovered = Vec::new();
    let mut total_sites = 0usize;
    for source in spawning_files() {
        let (sites, total) = uncovered_in(&source.rel, &source.text, aliases);
        uncovered.extend(sites);
        total_sites += total;
    }
    (uncovered, sources.len(), total_sites)
}

/// `uncovered_in` for a synthetic one-file tree.
fn uncovered_alone(text: &str) -> (Vec<Site>, usize) {
    uncovered_in("crates/synthetic/src/lib.rs", text, &BTreeSet::new())
}

#[test]
fn every_production_spawn_site_suppresses_its_console_window() {
    let (uncovered, _, _) = uncovered_sites();
    assert!(
        uncovered.is_empty(),
        "These spawn sites can run on a Windows user's machine and never pass \
         CREATE_NO_WINDOW, so each one flashes a black console window on screen.\n\n\
         Fix by calling `no_console_window(&mut cmd)` (or `_std` for \
         `std::process::Command`) beside the existing `strip_daemon_private_env` \
         call — the two things every agent-spawned child needs.\n\n\
         If the site genuinely cannot run on Windows, add it to EXEMPT in this \
         file with the reason.\n\n{}",
        uncovered
            .iter()
            .map(|s| format!("  {}:{}  {}", s.file, s.line, s.code))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// A walk that silently reads nothing reports a clean tree. Pin a floor.
#[test]
fn the_walk_actually_reads_the_tree() {
    let root = repo_root();
    let files = rust_sources(&root);
    assert!(
        files.len() > 400,
        "walked only {} .rs files under crates/*/src — the walk is broken, and a \
         broken walk reports every census as clean",
        files.len()
    );
    let (_, _, sites) = uncovered_sites();
    assert!(
        sites > 80,
        "found only {sites} `Command::new` sites; the tree has well over a \
         hundred. The scan is not seeing the code it claims to audit."
    );
}

/// Name the paths whose flashing the user actually reported, so a walk that
/// stops early — or an EXEMPT row that grows too broad — cannot quietly drop
/// them from the census.
#[test]
fn the_known_hot_paths_are_covered() {
    let root = repo_root();
    for (rel, needle) in [
        // `developer__shell`: one flash per shell tool call.
        (
            "crates/biorouter-mcp/src/developer/shell.rs",
            "no_console_window(&mut command_builder)",
        ),
        // Background jobs poll `tasklist` and kill with `taskkill`.
        (
            "crates/biorouter-mcp/src/developer/background.rs",
            "no_console_window",
        ),
        // Desktop control. This row used to name
        // `computercontroller/mod.rs`, which drove the desktop through
        // PowerShell. Built-in Biorouter Copilot replaced that: the file is now a
        // 128-line shim over `computer_use::SessionRuntime` and spawns nothing
        // at all, so the old row asserted a call in a file with no children to
        // prepare — a hot-path guard that could only ever pass vacuously.
        // The spawning moved here, and this runtime reaches the flag directly
        // through `creation_flags` rather than through the helper.
        (
            "crates/biorouter-mcp/src/computer_use/runtime.rs",
            "CREATE_NO_WINDOW",
        ),
        // The helper is started suspended and assigned to a job object before
        // it is resumed, so this file spawns too and needs the flag in its own
        // right.
        (
            "crates/biorouter-mcp/src/computer_use/windows_job.rs",
            "CREATE_NO_WINDOW",
        ),
        // Agent Drafter builds run node/npx on every app build.
        (
            "crates/biorouter-mcp/src/agent_drafter/mod.rs",
            "no_console_window_std",
        ),
    ] {
        let text = std::fs::read_to_string(root.join(rel))
            .unwrap_or_else(|e| panic!("{rel} must be readable: {e}"));
        assert!(
            text.contains(needle),
            "{rel} must prepare its children with `{needle}` — this is one of the \
             paths whose console flash was reported on Windows"
        );
    }
}

/// The census must recognise EVERY spelling of a test-only gate, not just the
/// bare `#[cfg(test)]`.
///
/// ⚠ This is a regression test for a real merge failure, and the failure mode
/// is worth stating because it is not the obvious one. The census and the
/// Claude Code shim fix were written on separate branches. Each passed alone.
/// The moment they met on `main`, the shim's `#[cfg(all(test, windows))] mod
/// windows_shim_tests` became invisible to a matcher that only knew
/// `#[cfg(test)]`, so a `std::process::Command` inside a `#[test]` function was
/// reported as a production console flash. A census that cries wolf gets
/// disabled, which costs the coverage it exists to provide.
#[test]
fn a_test_only_gate_is_recognised_however_it_is_spelled() {
    for spelling in [
        "#[cfg(test)]",
        "#[cfg(all(test, windows))]",
        "#[cfg(all(windows, test))]", // order must not matter
        "#[cfg(all(test, unix))]",
        "#[cfg(all(test, target_os = \"macos\"))]",
        "#[cfg(all(test, feature = \"aws-providers\"))]",
        "    #[cfg(all(test, windows))]", // indented
        "#[cfg(all(test, all(windows, feature = \"x\")))]", // nested
    ] {
        assert!(
            is_test_only_cfg(spelling),
            "`{spelling}` gates its module to test builds, so the census must \
             skip the spawn sites inside it"
        );
    }
}

/// The other direction, and the one that matters more: over-excluding is
/// SILENT. If this predicate ever returns true for a gate that also holds in a
/// normal build, the census stops reporting real production spawn sites and
/// nothing tells us.
#[test]
fn a_gate_that_also_holds_outside_tests_is_not_treated_as_test_only() {
    for spelling in [
        "#[cfg(windows)]",
        "#[cfg(unix)]",
        "#[cfg(not(test))]",                // the exact inverse
        "#[cfg(any(test, windows))]",       // holds on windows WITHOUT test
        "#[cfg(all(not(test), windows))]",  // holds only outside test
        "#[cfg(feature = \"test-utils\")]", // merely contains the word "test"
        "#[cfg(feature = \"integration-test\")]",
        "let x = 1;", // not an attribute at all
        "#[test]",    // a test fn, not a module gate
    ] {
        assert!(
            !is_test_only_cfg(spelling),
            "`{spelling}` can hold in a normal build, so a spawn site under it \
             is production code and must stay in the census"
        );
    }
}

/// End-to-end proof against the real file that broke, rather than a synthetic
/// string. A fixture only states what we think the tree looks like.
#[test]
fn the_real_windows_only_test_module_is_excluded_from_the_census() {
    let root = repo_root();
    let rel = "crates/biorouter/src/providers/coding_agent/discovery.rs";
    let text = std::fs::read_to_string(root.join(rel))
        .unwrap_or_else(|e| panic!("{rel} must be readable: {e}"));

    // Precondition: this test is meaningless if the module was renamed or
    // re-gated, so fail loudly rather than passing vacuously.
    let gate = "#[cfg(all(test, windows))]";
    let gate_at = text
        .lines()
        .position(|l| l.trim() == gate)
        .unwrap_or_else(|| {
            panic!("{rel} no longer contains a `{gate}` module — this test's premise is gone")
        });

    let spawn_at = text
        .lines()
        .position(|l| l.contains("std::process::Command::new(&resolved)"))
        .expect("the live shim test must still spawn the resolved binary");
    assert!(
        spawn_at > gate_at,
        "the spawn site must sit inside the gated module for this test to mean anything"
    );

    let marked = test_module_lines(&strip_literals(&text));
    assert!(
        marked.contains(&(spawn_at + 1)),
        "line {} of {rel} is inside `{gate}` and must be excluded from the census",
        spawn_at + 1
    );
}

/// The test-only regions in every file this census reads stay inside the scope
/// their gate is written in, judged by an independent reading of the braces.
/// A region that runs past its scope takes production spawn sites with it, and
/// the census then never looks at them.
#[test]
fn the_test_regions_it_reads_stay_inside_their_scope() {
    let mut failures = Vec::new();
    let files = spawning_files();
    for source in &files {
        for failure in region_containment_failures(&source.code) {
            failures.push(format!("{}: {failure}", source.rel));
        }
    }
    assert!(
        failures.is_empty(),
        "these test-only regions cover code outside their scope, so the spawn \
         sites there are never checked:\n{}",
        failures.join("\n")
    );
    assert!(files.len() > 40, "read only {} spawning files", files.len());
}

/// Spawn sites the previous reader silently skipped. Each text has exactly one
/// production `Command::new` with no preparation call anywhere near it, placed
/// where the old reader thought test code still was:
///
/// * after a gate with no body. The old reader had no `;` stop at all, and no
///   stop at a field's `,` or an enclosing `}`, so the region ran on through the
///   next balanced block.
/// * after a string continuation. The old stripper dropped the newline after a
///   `\` in a string, so every later region boundary moved up a line per
///   continuation, over the production line just before a test module.
/// * after `'\''`. The old stripper took `'\'` as a char literal and let the
///   stray quote open a string that blanked the test module's closing braces.
#[test]
fn spawn_sites_the_previous_reader_skipped_are_reported() {
    for (why, text) in [
        (
            "after an out-of-line test module",
            "#[cfg(test)]\nmod tests;\n\nfn spawn() {\n    let _ = std::process::Command::new(P);\n}\n",
        ),
        (
            "after a test-only `use`",
            "#[cfg(test)]\nuse std::fmt;\n\nfn spawn() {\n    let _ = std::process::Command::new(P);\n}\n",
        ),
        (
            "after a test-only struct field",
            "struct S {\n    #[cfg(test)]\n    a: u32,\n}\n\nimpl S {\n    fn spawn() {\n        \
             let _ = std::process::Command::new(P);\n    }\n}\n",
        ),
        (
            "after a test-only enum variant",
            "enum E {\n    A,\n    #[cfg(test)]\n    B,\n}\n\nfn spawn() {\n    \
             let _ = std::process::Command::new(P);\n}\n",
        ),
        (
            "after a test-only match arm",
            "fn spawn(e: E) {\n    match e {\n        E::A => {}\n        #[cfg(test)]\n        \
             E::B => return,\n    }\n    let _ = std::process::Command::new(P);\n}\n",
        ),
        (
            "after a test-only field in a struct literal",
            "fn s() -> Result<S, ()> {\n    Ok(S {\n        #[cfg(test)]\n        b: 2,\n    })\n}\n\n\
             fn spawn() {\n    let _ = std::process::Command::new(P);\n}\n",
        ),
        (
            "right before a test module, after a string continuation",
            "fn a() {\n    let s = \"one \\\n        two\";\n}\n\
             fn spawn() { let _ = std::process::Command::new(P); }\n\
             #[cfg(test)]\nmod tests {\n    fn t() {}\n}\n",
        ),
        (
            "after a test module holding `'\\''`",
            "#[cfg(test)]\nmod tests {\n    fn t() { let q = ('\\'', '\"'); }\n}\n\
             fn spawn() { let _ = std::process::Command::new(P); }\n\
             const X: &str = \"end\";\n",
        ),
    ] {
        let (uncovered, total) = uncovered_alone(text);
        assert_eq!(total, 1, "{why}: the fixture must hold exactly one spawn:\n{text}");
        assert_eq!(
            uncovered.len(),
            1,
            "{why}: the one production spawn must be reported, not read as test code:\n{text}"
        );
    }
    // And the same shapes, with the spawn inside the test code, stay exempt.
    for (why, text) in [
        (
            "inside a test module after a string continuation",
            "fn a() {\n    let s = \"one \\\n        two\";\n}\n\
             #[cfg(test)]\nmod tests {\n    fn t() { let _ = std::process::Command::new(P); }\n}\n",
        ),
        (
            "inside a test-only match arm with a block",
            "fn f(e: E) {\n    match e {\n        #[cfg(test)]\n        E::B => {\n            \
             let _ = std::process::Command::new(P);\n        }\n        _ => {}\n    }\n}\n",
        ),
    ] {
        let (uncovered, total) = uncovered_alone(text);
        assert_eq!(
            total, 1,
            "{why}: the fixture must hold exactly one spawn:\n{text}"
        );
        assert!(
            uncovered.is_empty(),
            "{why}: test code must stay exempt:\n{text}"
        );
    }
}

// ---------------------------------------------------------------------------
// Four ways the previous reading missed a production spawn, each with a site
// that must be flagged and a control that must not be. Each must-flag case was
// seen passing unflagged under the reading it replaced.
// ---------------------------------------------------------------------------

/// Why `text` does not have exactly `sites` spawn sites with `uncovered` of
/// them reported, if it does not.
fn census_mismatch(why: &str, text: &str, sites: usize, uncovered: usize) -> Option<String> {
    let (found, total) = uncovered_alone(text);
    ((total, found.len()) != (sites, uncovered)).then(|| {
        format!(
            "{why}: expected {sites} spawn site(s), {uncovered} of them reported; got {total} \
             site(s), {} reported {found:?}\n{text}",
            found.len()
        )
    })
}

#[track_caller]
fn assert_census(why: &str, text: &str, sites: usize, uncovered: usize) {
    if let Some(mismatch) = census_mismatch(why, text, sites, uncovered) {
        panic!("{mismatch}");
    }
}

/// Every case, checked before any is reported, so one run names them all.
#[track_caller]
fn assert_all_census(cases: &[(&str, String, usize, usize)]) {
    let mismatches: Vec<String> = cases
        .iter()
        .filter_map(|(why, text, sites, uncovered)| census_mismatch(why, text, *sites, *uncovered))
        .collect();
    assert!(
        mismatches.is_empty(),
        "{} of {} cases wrong:\n\n{}",
        mismatches.len(),
        cases.len(),
        mismatches.join("\n")
    );
}

/// 1. A gate on a match arm whose pattern starts with `crate::` covers that arm
///    only. The reader took `crate` for an item keyword, and an item does not
///    end at its `,`, so the region ran on through the next arm's block.
#[test]
fn a_gated_match_arm_on_a_crate_path_covers_that_arm_only() {
    assert_all_census(&[
        (
            "a spawn in the arm after a gated `crate::` arm",
            "fn spawn(e: E) {\n    match e {\n        #[cfg(test)]\n        crate::E::B => return,\n        \
             crate::E::A => {\n            let _ = std::process::Command::new(P);\n        }\n    }\n}\n"
                .to_string(),
            1,
            1,
        ),
        // Control: the spawn inside the gated arm is test code.
        (
            "a spawn inside a gated `crate::` arm",
            "fn spawn(e: E) {\n    match e {\n        #[cfg(test)]\n        crate::E::B => {\n            \
             let _ = std::process::Command::new(P);\n        }\n        crate::E::A => {}\n    }\n}\n"
                .to_string(),
            1,
            0,
        ),
        // The two other things that start with `crate` still end at their `;`.
        (
            "after a gated `extern crate`",
            "#[cfg(test)]\nextern crate foo;\nfn spawn() {\n    let _ = std::process::Command::new(P);\n}\n"
                .to_string(),
            1,
            1,
        ),
        (
            "after a gated `crate::m!();`",
            "fn spawn() {\n    #[cfg(test)]\n    crate::m!();\n    let _ = std::process::Command::new(P);\n}\n"
                .to_string(),
            1,
            1,
        ),
    ]);
}

/// 2. `new` on another name for `Command` is a spawn site. The previous rule
///    was the substring `Command::new`, which none of these contains, so none
///    was even counted.
#[test]
fn every_spelling_of_a_command_is_a_spawn_site() {
    let spellings = [
        (
            "a `use … as` alias",
            "use std::process::Command as Cmd;\nfn spawn() {\n    let c = Cmd::new(P);\n    PREPARE\n}\n",
        ),
        (
            "tokio's Command under an alias",
            "use tokio::process::Command as Child;\nfn spawn() {\n    let c = Child::new(P);\n    PREPARE\n}\n",
        ),
        (
            "an alias inside a use group",
            "use std::process::{Stdio, Command as C};\nfn spawn() {\n    let c = C::new(P);\n    PREPARE\n}\n",
        ),
        (
            "a qualified self type",
            "fn spawn() {\n    let c = <std::process::Command>::new(P);\n    PREPARE\n}\n",
        ),
        (
            "tokio's Command as a qualified self type",
            "fn spawn() {\n    let c = <tokio::process::Command>::new(P);\n    PREPARE\n}\n",
        ),
        (
            "a type alias",
            "type Cmd = std::process::Command;\nfn spawn() {\n    let c = Cmd::new(P);\n    PREPARE\n}\n",
        ),
        (
            "an alias of an alias, as a qualified self type",
            "use std::process::Command as A;\ntype B = A;\nfn spawn() {\n    let c = <B>::new(P);\n    PREPARE\n}\n",
        ),
        (
            "a raw identifier",
            "fn spawn() {\n    let c = std::process::Command::r#new(P);\n    PREPARE\n}\n",
        ),
    ];
    let mut cases = Vec::new();
    for (why, text) in spellings {
        cases.push((why, text.replace("PREPARE", ""), 1, 1));
        // Control: the same site, prepared, is covered.
        cases.push((
            why,
            text.replace("PREPARE", "no_console_window_std(&mut c);"),
            1,
            0,
        ));
    }
    assert_all_census(&cases);

    // An alias declared in one file and spelled bare in another.
    let tree = command_aliases(&[&code_of("pub use std::process::Command as Spawner;\n")]);
    let (found, total) = uncovered_in(
        "crates/synthetic/src/b.rs",
        "use crate::a::Spawner;\nfn spawn() {\n    let _ = Spawner::new(P);\n}\n",
        &tree,
    );
    assert_eq!((total, found.len()), (1, 1), "{tree:?} {found:#?}");

    // Controls: other types' `new` is not a spawn.
    for text in [
        "struct Cmd;\nfn f() {\n    let _ = Cmd::new();\n}\n",
        "fn f() {\n    let _ = CommandLine::new();\n    let _ = Command::new_line();\n}\n",
        "fn f(x: u64) -> u32 {\n    x as u32\n}\n",
    ] {
        assert_census("not a spawn", text, 0, 0);
    }
}

/// 3. A gate that excludes Windows counts only on the function the site is in.
///    The previous search took the nearest UNINDENTED `fn` above the site, so an
///    impl method was judged by the free function above its impl.
#[test]
fn a_windows_excluding_gate_counts_only_on_the_function_it_governs() {
    assert_all_census(&[
        (
            "a method below an unrelated unix-only function",
            "#[cfg(unix)]\nfn unix_only() {}\n\nimpl S {\n    fn spawn(&self) {\n        \
             let _ = std::process::Command::new(P);\n    }\n}\n"
                .to_string(),
            1,
            1,
        ),
        (
            "a spawn after a nested unix-only function",
            "fn f() {\n    #[cfg(unix)]\n    fn inner() {}\n    let _ = std::process::Command::new(P);\n}\n"
                .to_string(),
            1,
            1,
        ),
        // Controls: the method's own gate, however many attributes and doc
        // comments sit beside it, and the statement's own gate.
        (
            "a unix-only method",
            "impl S {\n    #[cfg(unix)]\n    fn spawn(&self) {\n        \
             let _ = std::process::Command::new(P);\n    }\n}\n"
                .to_string(),
            1,
            0,
        ),
        (
            "a unix-only method with other attributes",
            "impl S {\n    /// Docs.\n    #[cfg(target_os = \"macos\")]\n    #[allow(dead_code)]\n    \
             // a comment\n    pub(crate) async fn spawn(&self) {\n        \
             let _ = std::process::Command::new(P);\n    }\n}\n"
                .to_string(),
            1,
            0,
        ),
        (
            "a unix-only statement",
            "fn spawn() {\n    #[cfg(unix)]\n    let _ = std::process::Command::new(P);\n}\n".to_string(),
            1,
            0,
        ),
    ]);
}

/// 4. A comment that names a preparation helper prepares nothing. The previous
///    reading searched the raw text of the function, comments and strings
///    included.
#[test]
fn a_comment_naming_the_helper_does_not_cover_a_spawn() {
    let mut cases: Vec<(&str, String, usize, usize)> = [
        (
            "a line comment",
            "fn spawn() {\n    // no_console_window is not needed here\n    \
             let _ = std::process::Command::new(P);\n}\n",
        ),
        (
            "a block comment",
            "fn spawn() {\n    /* see no_console_window_std */\n    \
             let _ = std::process::Command::new(P);\n}\n",
        ),
        (
            "a doc comment on the function",
            "/// Calls no_console_window like every spawn should.\nfn spawn() {\n    \
             let _ = std::process::Command::new(P);\n}\n",
        ),
        (
            "a string",
            "fn spawn() {\n    let note = \"creation_flags\";\n    \
             let _ = std::process::Command::new(P);\n}\n",
        ),
    ]
    .into_iter()
    .map(|(why, text)| (why, text.to_string(), 1, 1))
    .collect();
    // Control: the call itself.
    cases.push((
        "the real call",
        "fn spawn() {\n    let mut c = std::process::Command::new(P);\n    \
         no_console_window_std(&mut c); // and a comment\n}\n"
            .to_string(),
        1,
        0,
    ));
    assert_all_census(&cases);
}

/// 5. Five more spellings a reviewer walked past the line-by-line reading.
///    Each must-flag case below was seen unreported under it: `Self` in an
///    impl of `Command` was not a name it knew, it allowed one `>` and no `)`
///    or `as` between the type and `::new`, no turbofish, it read one line at
///    a time, and a gate that shared its line with a statement of its own
///    exempted the spawn below it.
#[test]
fn the_spellings_the_line_reader_missed_are_spawn_sites() {
    let cases: Vec<(&str, String, usize, usize)> = vec![
        // `Self` in an impl of `Command`.
        (
            "`Self::new` in an impl for std's Command",
            "pub trait Ext { fn make(p: &str) -> Self; }\nimpl Ext for std::process::Command {\n    \
             fn make(p: &str) -> Self {\n        Self::new(p)\n    }\n}\n"
                .to_string(),
            1,
            1,
        ),
        (
            "`<Self>::new` in an impl for tokio's Command",
            "impl Ext for tokio::process::Command {\n    fn make(p: &str) -> Self {\n        \
             <Self>::new(p)\n    }\n}\n"
                .to_string(),
            1,
            1,
        ),
        (
            "`Self::new` in a generic impl with a where clause",
            "impl<T> Ext<T> for Command\nwhere\n    T: Sized,\n{\n    fn make(p: &str) -> Self {\n        \
             Self::new(p)\n    }\n}\n"
                .to_string(),
            1,
            1,
        ),
        (
            "`Self::new` in an unsafe impl for an alias of Command",
            "use std::process::Command as Cmd;\nunsafe impl Ext for Cmd {\n    fn make(p: &str) -> Self {\n        \
             Self::new(p)\n    }\n}\n"
                .to_string(),
            1,
            1,
        ),
        // A qualified self type holding more than the type.
        (
            "a parenthesised qualified self type",
            "fn spawn() {\n    let c = <(std::process::Command)>::new(P);\n}\n".to_string(),
            1,
            1,
        ),
        (
            "a qualified path through a trait",
            "fn spawn() {\n    let c = <std::process::Command as Ext>::new(P);\n}\n".to_string(),
            1,
            1,
        ),
        // A turbofish.
        (
            "an empty turbofish",
            "fn spawn() {\n    let c = std::process::Command::<>::new(P);\n}\n".to_string(),
            1,
            1,
        ),
        // Split across lines.
        (
            "`Command` and `::new` on two lines",
            "fn spawn() {\n    let c = std::process::Command\n        ::new(P);\n}\n".to_string(),
            1,
            1,
        ),
        (
            "a qualified self type over three lines",
            "fn spawn() {\n    let c = <\n        std::process::Command\n    >::new(P);\n}\n".to_string(),
            1,
            1,
        ),
        // A gate that governs a statement of its own.
        (
            "a gated statement on the line above",
            "fn spawn() {\n    #[cfg(unix)] let _ = ();\n    let _ = std::process::Command::new(P);\n}\n"
                .to_string(),
            1,
            1,
        ),
        (
            "a gated `use` on the line above the function",
            "#[cfg(unix)] use std::fmt;\nfn spawn() {\n    let _ = std::process::Command::new(P);\n}\n"
                .to_string(),
            1,
            1,
        ),
        // ── Controls ──
        (
            "`Self::new` in an impl for Command that prepares it",
            "impl Ext for std::process::Command {\n    fn make(p: &str) -> Self {\n        \
             let mut c = Self::new(p);\n        no_console_window_std(&mut c);\n        c\n    }\n}\n"
                .to_string(),
            1,
            0,
        ),
        (
            "a split call that prepares its command",
            "fn spawn() {\n    let mut c = std::process::Command\n        ::new(P);\n    \
             no_console_window_std(&mut c);\n}\n"
                .to_string(),
            1,
            0,
        ),
        (
            "`Self::new` in an impl for another type",
            "impl Ext for Other {\n    fn make() -> Self {\n        Self::new()\n    }\n}\n".to_string(),
            0,
            0,
        ),
        (
            "`Self` in an impl nested inside an impl for Command names the inner type",
            "impl Ext for std::process::Command {\n    fn make(p: &str) -> Self {\n        struct S;\n        \
             impl S {\n            fn new() -> Self { S }\n            fn f() -> Self { Self::new() }\n        }\n        \
             unimplemented!()\n    }\n}\n"
                .to_string(),
            0,
            0,
        ),
        (
            "an `impl Trait` return type naming Command is not an impl of it",
            "impl Other {\n    fn g() -> impl Fn() -> std::process::Command {\n        let _ = Self::new();\n        \
             || todo!()\n    }\n}\n"
                .to_string(),
            0,
            0,
        ),
        (
            "`new` on a collection of commands is not a spawn",
            "fn f() {\n    let _ = Vec::<std::process::Command>::new();\n    \
             let _ = <Vec<Command>>::new();\n    let _ = HashMap::<String, Command>::new();\n}\n"
                .to_string(),
            0,
            0,
        ),
        (
            "`use … as` is not a call",
            "use std::process::Command as Cmd;\nfn f() {}\n".to_string(),
            0,
            0,
        ),
        (
            "a line of attributes only still gates the spawn below it",
            "fn spawn() {\n    #[cfg(unix)] #[allow(unused)]\n    let _ = std::process::Command::new(P);\n}\n"
                .to_string(),
            1,
            0,
        ),
    ];
    assert_all_census(&cases);

    // A split site is reported on the line its type is on.
    let (found, _) =
        uncovered_alone("fn spawn() {\n    let c = std::process::Command\n        ::new(P);\n}\n");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].line, 2, "{found:#?}");
}

/// 6. Two more reader defects. A keyword before `<` was read as a path the
///    `<` gives generic arguments to, so `return <Command>::new(p)` looked like
///    `Vec::<Command>::new()` and was not a site. And a higher-ranked bound in a
///    `where` clause (`for<'a> Fn(&'a str)`) was read as the impl's `for`,
///    which put the self type's start after its end and panicked on the slice,
///    turning every census test red on legitimate code.
#[test]
fn keywords_before_a_qualified_self_and_higher_ranked_bounds_are_read() {
    let cases: Vec<(&str, String, usize, usize)> = vec![
        // ── Must flag: a keyword, not a path, before the `<` ──
        (
            "`return <Command>::new`",
            "fn spawn() -> Command {\n    return <Command>::new(P);\n}\n".to_string(),
            1,
            1,
        ),
        (
            "`if <Command>::new`",
            "fn spawn() {\n    if <Command>::new(P).status().is_ok() {}\n}\n".to_string(),
            1,
            1,
        ),
        (
            "`&mut <Command>::new`",
            "fn spawn() {\n    f(&mut <Command>::new(P));\n}\n".to_string(),
            1,
            1,
        ),
        (
            "`match <std::process::Command>::new`",
            "fn spawn() {\n    match <std::process::Command>::new(P).status() {\n        _ => {}\n    }\n}\n"
                .to_string(),
            1,
            1,
        ),
        (
            "`in <Command>::new`",
            "fn spawn() {\n    for a in <Command>::new(P).get_args() {}\n}\n".to_string(),
            1,
            1,
        ),
        (
            "`break <Command>::new`",
            "fn spawn() -> Command {\n    loop {\n        break <Command>::new(P);\n    }\n}\n".to_string(),
            1,
            1,
        ),
        (
            "`while <Command>::new`",
            "fn spawn() {\n    while <Command>::new(P).status().is_err() {}\n}\n".to_string(),
            1,
            1,
        ),
        (
            "`return <Self>::new` in an impl for Command",
            "impl Ext for std::process::Command {\n    fn make(p: &str) -> Self {\n        \
             return <Self>::new(p);\n    }\n}\n"
                .to_string(),
            1,
            1,
        ),
        // ── Must flag: a higher-ranked bound in the where clause ──
        (
            "`Self::new` in an impl for Command with a higher-ranked where bound",
            "impl<F> Ext<F> for std::process::Command\nwhere\n    F: for<'a> Fn(&'a str),\n{\n    \
             fn make(p: &str) -> Self {\n        Self::new(p)\n    }\n}\n"
                .to_string(),
            1,
            1,
        ),
        (
            "`Self::new` in an impl for Command with a higher-ranked where predicate",
            "impl<T> Ext for Command\nwhere\n    for<'a> &'a T: Sized,\n{\n    \
             fn make(p: &str) -> Self {\n        Self::new(p)\n    }\n}\n"
                .to_string(),
            1,
            1,
        ),
        (
            "`Self::new` in an impl for Command with a higher-ranked bound in its generics",
            "impl<F: for<'a> Fn(&'a str)> Ext<F> for Command {\n    fn make(p: &str) -> Self {\n        \
             Self::new(p)\n    }\n}\n"
                .to_string(),
            1,
            1,
        ),
        // ── Must pass ──
        (
            "a keyword before a collection of commands is still not a spawn",
            "fn f() {\n    return Vec::<Command>::new();\n}\nfn g() {\n    \
             if <Vec<Command>>::new().is_empty() {}\n}\n"
                .to_string(),
            0,
            0,
        ),
        (
            "`return <Command>::new` that prepares its command",
            "fn spawn() -> Command {\n    let mut c = <Command>::new(P);\n    \
             no_console_window_std(&mut c);\n    return c;\n}\n"
                .to_string(),
            1,
            0,
        ),
        (
            "a where bound naming Command does not make the impl one of Command",
            "impl<F> Ext<F> for Other\nwhere\n    F: for<'a> Fn(&'a str) -> Command,\n{\n    \
             fn make() -> Self {\n        Self::new()\n    }\n}\n"
                .to_string(),
            0,
            0,
        ),
        (
            "an inherent impl with a higher-ranked where bound is read, not panicked on",
            "impl<F> Other<F>\nwhere\n    F: for<'a> Fn(&'a str),\n{\n    fn new() -> Self {\n        \
             todo!()\n    }\n    fn f() -> Self {\n        Self::new()\n    }\n}\n"
                .to_string(),
            0,
            0,
        ),
    ];
    assert_all_census(&cases);
}

// ---------------------------------------------------------------------------
// The reader itself (`census_support/rust_source.rs`)
// ---------------------------------------------------------------------------

/// What the region reader makes of each shape a test-only gate is written on.
/// The bodiless shapes are the ones that used to run on.
#[test]
fn the_region_reader_ends_each_gated_shape_where_it_ends() {
    for (why, text, want) in [
        (
            "a struct field",
            "struct S {\n    #[cfg(test)]\n    a: u32,\n    b: u32,\n}\n",
            (2, 3, RegionKind::Member, RegionEnd::Comma),
        ),
        (
            "a last field with no trailing comma: the `}` is not part of it",
            "struct S {\n    #[cfg(test)]\n    a: u32\n}\nfn f() {}\n",
            (2, 3, RegionKind::Member, RegionEnd::EnclosingClose),
        ),
        (
            "an enum variant",
            "enum E {\n    #[cfg(test)]\n    B,\n}\n",
            (2, 3, RegionKind::Member, RegionEnd::Comma),
        ),
        (
            "an enum variant with fields over several lines",
            "enum E {\n    #[cfg(test)]\n    B {\n        x: u32,\n    },\n    C,\n}\n",
            (2, 5, RegionKind::Member, RegionEnd::BodyClosed),
        ),
        (
            "a match arm",
            "match e {\n    #[cfg(test)]\n    E::B => return Ok(()),\n    _ => {}\n}\n",
            (2, 3, RegionKind::Member, RegionEnd::Comma),
        ),
        (
            "a match arm whose pattern starts with `crate::`",
            "match e {\n    #[cfg(test)]\n    crate::E::B => return,\n    crate::E::A => {\n        \
             x();\n    }\n}\n",
            (2, 3, RegionKind::Member, RegionEnd::Comma),
        ),
        (
            "a match arm whose call spans lines",
            "match e {\n    #[cfg(test)]\n    E::B => f(\n        a,\n        b,\n    ),\n    _ => {}\n}\n",
            (2, 6, RegionKind::Member, RegionEnd::Comma),
        ),
        (
            "a last match arm with no trailing comma",
            "match e {\n    _ => {}\n    #[cfg(test)]\n    E::B => 1\n}\n",
            (3, 4, RegionKind::Member, RegionEnd::EnclosingClose),
        ),
        (
            "a field in a struct literal",
            "Ok(S {\n    #[cfg(test)]\n    b: IoProbe::default(),\n})\n",
            (2, 3, RegionKind::Member, RegionEnd::Comma),
        ),
        (
            "a field named like a contextual keyword",
            "struct S {\n    #[cfg(test)]\n    union: u32,\n    b: u32,\n}\n",
            (2, 3, RegionKind::Member, RegionEnd::Comma),
        ),
        (
            "an out-of-line module",
            "#[cfg(test)]\nmod tests;\nfn f() {}\n",
            (1, 2, RegionKind::Module, RegionEnd::Semicolon),
        ),
        (
            "a module with a body",
            "#[cfg(test)]\nmod tests {\n    fn a() {}\n    fn b() {}\n}\nfn f() {}\n",
            (1, 5, RegionKind::Module, RegionEnd::BodyClosed),
        ),
        (
            "a `use`",
            "#[cfg(test)]\nuse std::fmt;\nfn f() {}\n",
            (1, 2, RegionKind::Item, RegionEnd::Semicolon),
        ),
        (
            "an `extern crate`",
            "#[cfg(test)]\nextern crate foo;\nfn f() {}\n",
            (1, 2, RegionKind::Item, RegionEnd::Semicolon),
        ),
        (
            "a macro called through `crate::`",
            "fn g() {\n    #[cfg(test)]\n    crate::m!();\n    x();\n}\n",
            (2, 3, RegionKind::Member, RegionEnd::Semicolon),
        ),
        (
            "a fn with a where clause",
            "#[cfg(test)]\npub(crate) async fn f<T>(t: T)\nwhere\n    T: Clone,\n{\n    x();\n}\nfn g() {}\n",
            (1, 7, RegionKind::Function, RegionEnd::BodyClosed),
        ),
        (
            "a const over several lines",
            "#[cfg(test)]\nconst X: &[u8] = &[\n    1,\n    2,\n];\nfn g() {}\n",
            (1, 5, RegionKind::Item, RegionEnd::Semicolon),
        ),
    ] {
        let regions = test_regions(&strip_literals(text));
        let (start, end, kind, ended_by) = want;
        assert!(
            regions.first().is_some_and(|r| r.start == start
                && r.end == end
                && r.kind == kind
                && r.ended_by == ended_by),
            "{why}: expected lines {start}..={end}, {kind:?}, {ended_by:?}; got {regions:#?}\n{text}"
        );
        assert!(
            region_containment_failures(&strip_literals(text)).is_empty(),
            "{why}: {:#?}",
            region_containment_failures(&strip_literals(text))
        );
    }
}

/// The containment control catches a region that runs out of its scope. The
/// reader no longer produces one, so it is handed the region an older reader
/// produced for this text (lines 2 to 7: the field, the struct's `}` and all of
/// `impl S`), and must reject it for the right reason.
#[test]
fn the_containment_check_is_not_vacuous() {
    let text = "struct S {\n    #[cfg(test)]\n    a: u32,\n}\nimpl S {\n    fn f() {}\n}\n";
    let stripped = strip_literals(text);
    assert!(region_containment_failures(&stripped).is_empty());
    let what_the_old_reader_read = rust_source::TestRegion {
        start: 2,
        end: 7,
        kind: RegionKind::Member,
        ended_by: RegionEnd::BodyClosed,
    };
    let failures = rust_source::containment_failures(&stripped, &[what_the_old_reader_read]);
    assert_eq!(failures.len(), 1, "{failures:#?}");
    assert!(
        failures[0].contains("runs to line 7, past the `}` on line 4"),
        "{failures:#?}"
    );

    // And a gate that never finds its end is reported too.
    let text = "#[cfg(test)]\nstatic X: u8 = 1\n";
    let failures = region_containment_failures(&strip_literals(text));
    assert_eq!(failures.len(), 1, "{failures:#?}");
    assert!(failures[0].contains("end of the file"), "{failures:#?}");
}

/// Files where reviewers measured an over-exemption, pinned by production lines
/// that an older reader read as test code. Each file's own test module must
/// still be read as one, or the pin would pass for the wrong reason.
/// Every `EXEMPT` row still names a path that exists.
///
/// A row whose file was renamed or deleted stops describing the tree and starts
/// pre-authorising whatever is written at that path next — the permission
/// outlives the reason for it, and nothing says so. Two such rows were found
/// here, both pointing at `computercontroller/platform/`, a directory that no
/// longer exists.
#[test]
fn every_exemption_still_names_something_real() {
    let root = repo_root();
    let dead: Vec<&str> = EXEMPT
        .iter()
        .map(|(path, _)| *path)
        .filter(|path| !root.join(path).exists())
        .collect();
    assert!(
        dead.is_empty(),
        "these EXEMPT rows name paths that do not exist, so they exempt nothing today \
         and will silently exempt whatever is created there tomorrow: {dead:?}"
    );
}

#[test]
fn the_measured_over_exemptions_are_gone() {
    let root = repo_root();
    for (rel, production, test) in [
        (
            "crates/biorouter-mcp/src/developer/rmcp_developer.rs",
            &[
                "    fn get_info(&self) -> ServerInfo {",
                "impl ServerHandler for DeveloperServer {",
            ][..],
            "    fn get_info_reports_a_missing_working_directory() {",
        ),
        (
            "crates/biorouter/src/config/base.rs",
            &[
                "enum SecretStorage {",
                "pub trait ConfigValue {",
                "    pub fn exists(&self) -> bool {",
            ][..],
            "mod tests {",
        ),
        (
            "crates/biorouter-cli/src/commands/schedule.rs",
            &[
                "fn needs_a_person(action: &str) -> String {",
                "        needs_terminal::require(terminal, &needs_a_person(action))?;",
            ][..],
            "mod tests {",
        ),
    ] {
        let text = std::fs::read_to_string(root.join(rel))
            .unwrap_or_else(|e| panic!("{rel} must be readable: {e}"));
        let in_test = test_module_lines(&strip_literals(&text));
        let line_of = |needle: &str| {
            text.lines()
                .position(|l| l.starts_with(needle))
                .map(|i| i + 1)
                .unwrap_or_else(|| {
                    panic!("{rel} no longer has a line starting `{needle}`; re-pin this row")
                })
        };
        for needle in production {
            let line = line_of(needle);
            assert!(
                !in_test.contains(&line),
                "{rel}:{line} `{}` is production code and must not be read as test code",
                needle.trim()
            );
        }
        let line = line_of(test);
        assert!(
            in_test.contains(&line),
            "{rel}:{line} `{}` is inside the file's test module and must be read as test code",
            test.trim()
        );
    }
}

/// The reading blanks text but must never move it: a site is reported, and a
/// `#[cfg(test)]` boundary is read, by line number. Checked over every file the
/// census walks, since a fixture only covers the spellings someone thought of.
#[test]
fn reading_the_code_keeps_every_line_where_it_was() {
    let (sources, _) = tree();
    let moved: Vec<&str> = sources
        .iter()
        .filter(|s| s.code.matches('\n').count() != s.text.matches('\n').count())
        .map(|s| s.rel.as_str())
        .collect();
    assert!(
        moved.is_empty(),
        "reading changed the line count of: {moved:#?}"
    );

    // A string continuation, then a spawn: reported on its real line.
    let text = "fn f() {\n    let s = \"one \\\n        two\";\n    \
                let _ = std::process::Command::new(P);\n}\n";
    let (found, _) = uncovered_alone(text);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].line, 4, "{found:#?}");
}

/// Char literals that contain a quote, a backslash or a brace must not open a
/// string or a block that is not there.
#[test]
fn char_literals_do_not_hide_the_code_after_them() {
    for text in [
        "fn f() { let q = ('\\'', '\"'); let _ = std::process::Command::new(P); }\n",
        "fn f() { let q = ['\\\\', '\"']; let _ = std::process::Command::new(P); }\n",
        "fn f<'a>(x: &'a str) { let b = '{'; let _ = std::process::Command::new(P); }\n",
        "fn f() { let e = '\\u{1F600}'; let q = '\"'; let _ = std::process::Command::new(P); }\n",
    ] {
        assert_census(
            "the spawn after these char literals must still be seen",
            text,
            1,
            1,
        );
    }
    // And the brace in `'}'` must not close a test module early.
    assert_census(
        "a spawn in a test module holding `'}'`",
        "#[cfg(test)]\nmod tests {\n    const C: char = '}';\n    \
         fn f() { let _ = std::process::Command::new(P); }\n}\n",
        1,
        0,
    );
}
