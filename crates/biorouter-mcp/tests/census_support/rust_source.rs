//! The Rust source reader behind the console-window census
//! (`../no_console_window_census.rs`, which includes this file by path): a
//! comment and literal stripper that never moves a line, and a reader for the
//! regions a test-only `cfg` gate covers.
//!
//! It was written for two text censuses. The other one, of inheritable socket
//! binds, is gone: a text scanner cannot resolve Rust paths, and a reviewer
//! walked four spellings of a tokio bind past it, so that check is now a
//! compiler lint (`scripts/check-non-inheritable-sockets.sh`). The console
//! census stays textual only because its compiler equivalent would need an
//! allow attribute at every correctly prepared spawn site in the tree.
//!
//! It errs toward reporting. An over-report is visible and fixable. An
//! under-report is silent, and every bug recorded below was of that kind.

// The census uses a subset of what is here; the rest is its own tests' view.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Every `.rs` file under `crates/*/src`, sorted.
pub fn rust_sources(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    for crate_dir in std::fs::read_dir(root.join("crates"))
        .expect("crates/ must exist")
        .flatten()
    {
        let src = crate_dir.path().join("src");
        if src.is_dir() {
            walk(&src, &mut out);
        }
    }
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Stripping comments and literals
// ---------------------------------------------------------------------------

/// Blank out comments, string literals and char literals, keeping every
/// character's line position. Code inside a comment or a string is not code,
/// and a brace inside one must not be counted.
///
/// ⚠ Each rule below replaced one that was observed failing in this tree:
///
/// * `agent_drafter/render.rs` generates shell and JavaScript, so its raw
///   strings are full of unbalanced braces. Counting them ended its test module
///   hundreds of lines early.
/// * A `\` at the end of a line inside a string (a continuation, all over this
///   tree) used to be skipped together with the newline after it. Every one of
///   them moved each later line up by one, in 256 files: a site on line 496 was
///   reported at 480, and a `#[cfg(test)]` boundary moved with it.
/// * Reading "a quote within three chars" as a char literal takes `'\''` as the
///   three chars `'\'` and leaves its last quote behind to open a literal of its
///   own. In `('\'', '"')` that stray quote swallows `, '` and exposes the `"`,
///   which then blanks everything up to the next `"` in the file.
pub fn strip_literals(text: &str) -> String {
    let bytes: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let keep_line = |out: &mut String, c: char| out.push(if c == '\n' { '\n' } else { ' ' });

        // Raw string: r"..." / r#"..."# / r##"..."##
        if c == 'r' && i + 1 < bytes.len() && (bytes[i + 1] == '"' || bytes[i + 1] == '#') {
            let mut j = i + 1;
            let mut hashes = 0;
            while j < bytes.len() && bytes[j] == '#' {
                hashes += 1;
                j += 1;
            }
            if j < bytes.len() && bytes[j] == '"' {
                out.push(' ');
                let close: String = std::iter::once('"')
                    .chain(std::iter::repeat_n('#', hashes))
                    .collect();
                j += 1;
                while j < bytes.len() {
                    if bytes[j] == '"'
                        && bytes[j..].iter().take(close.len()).collect::<String>() == close
                    {
                        j += close.len();
                        break;
                    }
                    keep_line(&mut out, bytes[j]);
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        if c == '"' {
            out.push(' ');
            i += 1;
            while i < bytes.len() {
                if bytes[i] == '\\' {
                    // Blank the escape AND what it escapes, keeping a newline if
                    // that is what follows (a string continuation).
                    keep_line(&mut out, bytes[i]);
                    if let Some(&next) = bytes.get(i + 1) {
                        keep_line(&mut out, next);
                    }
                    i += 2;
                    continue;
                }
                if bytes[i] == '"' {
                    i += 1;
                    break;
                }
                keep_line(&mut out, bytes[i]);
                i += 1;
            }
            continue;
        }
        // Char literal: `'x'`, or an escape such as `'\''`, `'\\'`, `'\n'` or
        // `'\u{1F600}'`. A lifetime (`'a`, `'static`) has no closing quote where
        // a char literal's would be, so it is left alone.
        if c == '\'' {
            let close = if bytes.get(i + 1) == Some(&'\\') {
                (i + 3..bytes.len().min(i + 12)).find(|&k| bytes[k] == '\'')
            } else if bytes.get(i + 2) == Some(&'\'') {
                Some(i + 2)
            } else {
                None
            };
            if let Some(end) = close {
                for &skipped in &bytes[i..=end] {
                    keep_line(&mut out, skipped);
                }
                i = end + 1;
                continue;
            }
        }
        if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == '/' {
            while i < bytes.len() && bytes[i] != '\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == '*' {
            let mut depth = 1;
            out.push_str("  ");
            i += 2;
            while i < bytes.len() && depth > 0 {
                if bytes[i] == '/' && i + 1 < bytes.len() && bytes[i + 1] == '*' {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                    continue;
                }
                if bytes[i] == '*' && i + 1 < bytes.len() && bytes[i + 1] == '/' {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    continue;
                }
                keep_line(&mut out, bytes[i]);
                i += 1;
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Test-only `cfg` gates
// ---------------------------------------------------------------------------

/// Split a `cfg` predicate list on the commas at nesting depth zero, so
/// `all(test, any(a, b))` yields `["test", "any(a, b)"]`.
pub fn split_top_level(list: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in list.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                // `get`, not `[..]`: clippy::string_slice. The indices come from
                // `char_indices`, so they are boundaries by construction.
                if let Some(part) = list.get(start..i) {
                    parts.push(part.trim());
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if let Some(rest) = list.get(start..) {
        parts.push(rest.trim());
    }
    parts
}

/// Whether a `cfg` predicate holds ONLY in a test build. `all(...)` is when any
/// arm is, because every arm must hold. `any(...)` and `not(...)` are
/// deliberately not: an item gated that way also exists in a normal build, so
/// the code under it is production code and must stay in a census. Erring that
/// way keeps this function's mistakes visible (an over-report) rather than
/// silent (an under-report).
pub fn cfg_is_test_only(predicate: &str) -> bool {
    let predicate = predicate.trim();
    if predicate == "test" {
        return true;
    }
    match predicate
        .strip_prefix("all(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        Some(inner) => split_top_level(inner).into_iter().any(cfg_is_test_only),
        None => false,
    }
}

/// Whether a line is an attribute gating the item below it to test builds.
/// Any spelling: `#[cfg(test)]`, `#[cfg(all(test, windows))]`, indented.
///
/// ⚠ Matching the literal `#[cfg(test)]` is not enough, and it fails in a way
/// that looked harmless and was not. The console census and the Claude Code
/// shim's `#[cfg(all(test, windows))] mod windows_shim_tests` were written on
/// separate branches and each passed alone; on `main` the module was invisible
/// to a matcher that knew one spelling, so a spawn in a `#[test]` was reported
/// as a production console flash. A census that cries wolf gets disabled.
pub fn is_test_only_cfg(line: &str) -> bool {
    line.trim_start()
        .strip_prefix("#[cfg(")
        .and_then(|rest| rest.strip_suffix(")]"))
        .is_some_and(cfg_is_test_only)
}

/// What a test-only gate is attached to, read from the first line of code after
/// it (stacked attributes skipped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    /// `mod tests { .. }` or `mod tests;`.
    Module,
    /// A function, with or without a body.
    Function,
    /// Any other item or statement that starts with a keyword: `impl`,
    /// `struct`, `enum`, `trait`, `use`, `const`, `static`, `type`, `let`,
    /// `macro_rules!`, `extern crate`.
    ///
    /// ⚠ Not `crate` on its own. `crate::` also starts a PATH, and a match arm
    /// whose pattern is `crate::E::B => …,` is a member that ends at its `,`.
    /// Read as an item, its region ran through the next arm's block and
    /// exempted the spawn there. `extern crate x;` and `crate::m!();` end at
    /// their `;` either way.
    Item,
    /// Anything else: a struct field, an enum variant, a match arm, a field in
    /// a struct literal, a macro call, an expression. These end at their own
    /// `,`, which items do not (a `where` clause is full of them).
    Member,
}

/// Why a region ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionEnd {
    /// The body it opened closed.
    BodyClosed,
    /// A `;` at nesting depth zero, before any body: `mod tests;`, `use x;`.
    Semicolon,
    /// A line ending in `,` at nesting depth zero, before any body: a member.
    Comma,
    /// A brace, parenthesis or bracket closed that the gated code never opened:
    /// the gate was on the last member of an enclosing list with no trailing
    /// comma, and the enclosing scope ended.
    EnclosingClose,
    /// The file ended first. Never right: see `region_containment_failures`.
    EndOfFile,
}

/// The lines one test-only gate covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestRegion {
    /// 1-based line of the `#[cfg(...)]` attribute.
    pub start: usize,
    /// 1-based last line inside the region, inclusive.
    pub end: usize,
    pub kind: RegionKind,
    pub ended_by: RegionEnd,
}

/// `kw` as a whole word at the start of `s`, and what follows it.
fn strip_keyword<'a>(s: &'a str, kw: &str) -> Option<&'a str> {
    let rest = s.strip_prefix(kw)?;
    if rest
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
    {
        return None;
    }
    Some(rest.trim_start())
}

fn kind_of_code(code: &str) -> RegionKind {
    let mut rest = code.trim_start();
    if let Some(after) = strip_keyword(rest, "pub") {
        rest = match after.strip_prefix('(') {
            Some(scoped) => scoped.split_once(')').map_or("", |(_, r)| r).trim_start(),
            None => after,
        };
    }
    // `extern crate x;`, before `extern` is taken for a qualifier below.
    if strip_keyword(rest, "extern").is_some_and(|after| strip_keyword(after, "crate").is_some()) {
        return RegionKind::Item;
    }
    // Qualifiers in front of `fn` (and `impl`, for `unsafe impl`). `extern
    // "C"` reaches here with its ABI string already blanked to spaces.
    loop {
        let before = rest;
        for qualifier in ["default", "async", "unsafe", "extern"] {
            if let Some(after) = strip_keyword(rest, qualifier) {
                rest = after;
            }
        }
        if rest == before {
            break;
        }
    }
    if strip_keyword(rest, "mod").is_some() {
        return RegionKind::Module;
    }
    if strip_keyword(rest, "fn").is_some()
        || strip_keyword(rest, "const").is_some_and(|after| strip_keyword(after, "fn").is_some())
    {
        return RegionKind::Function;
    }
    // Reserved words, so none of them can be a field or a variant. `union` is
    // contextual (a field may be called `union`), so it counts only when a name
    // follows.
    const ITEM_KEYWORDS: &[&str] = &[
        "impl",
        "struct",
        "enum",
        "trait",
        "type",
        "use",
        "const",
        "static",
        "let",
        "macro_rules",
    ];
    if ITEM_KEYWORDS
        .iter()
        .any(|kw| strip_keyword(rest, kw).is_some())
        || strip_keyword(rest, "union")
            .is_some_and(|after| after.starts_with(|c: char| c.is_alphabetic() || c == '_'))
    {
        return RegionKind::Item;
    }
    RegionKind::Member
}

/// Remove attributes from the front of a line. `Err(depth)` when one is still
/// open at the end of the line, with the bracket depth left to close.
fn skip_leading_attributes(line: &str) -> Result<&str, i32> {
    let mut rest = line.trim_start();
    while rest.starts_with('#') {
        let mut depth = 0i32;
        let mut closed_at = None;
        for (i, c) in rest.char_indices() {
            match c {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        closed_at = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        match closed_at {
            Some(i) => rest = rest.get(i + 1..).unwrap_or("").trim_start(),
            None => return Err(depth),
        }
    }
    Ok(rest)
}

/// The kind of the code a gate on line `attr` (0-based) is attached to.
fn region_kind(lines: &[&str], attr: usize) -> RegionKind {
    let mut open_attribute = 0i32;
    for line in lines.iter().skip(attr + 1) {
        if open_attribute > 0 {
            open_attribute += count(line, &['[']) - count(line, &[']']);
            continue;
        }
        match skip_leading_attributes(line) {
            Ok("") => continue,
            Ok(code) => return kind_of_code(code),
            Err(depth) => open_attribute = depth,
        }
    }
    RegionKind::Member
}

fn count(line: &str, chars: &[char]) -> i32 {
    line.chars().filter(|c| chars.contains(c)).count() as i32
}

/// Every region a test-only gate covers, in order, read from STRIPPED text.
///
/// ⚠ Two brace-matchers came before this one, and both were wrong in the
/// direction that makes a census under-report:
///
/// * The first closed the region the moment depth returned to zero on any line
///   with a `}`, so a one-line `fn f() {}` ended a whole test module and the
///   rest of it read as production. So: enter on the first `{`, and only then
///   let a return to zero close it.
/// * The second did that, and ignored what happens BEFORE a body is entered.
///   A gate on a struct field, an enum variant, a match arm or a struct-literal
///   field has no body. The enclosing `}` drove the depth negative, the region
///   ran on, and it closed only when the depth next came back to zero: at the
///   end of whatever balanced block came next. Measured on 2026-09-21: 371
///   production lines in four files read as test code, including every line of
///   `ServerHandler::get_info` in `developer/rmcp_developer.rs`, where a
///   reviewer put a tokio bind and watched the listener census pass.
///
/// So, before a body is entered, a region also ends at a `;` at nesting depth
/// zero, at a line ending in `,` at nesting depth zero when the gated code is a
/// member rather than an item, and as soon as anything closes that the gated
/// code never opened. `region_containment_failures` checks the result against
/// an independent reading of the braces.
pub fn test_regions(stripped: &str) -> Vec<TestRegion> {
    let lines: Vec<&str> = stripped.lines().collect();
    let mut regions = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !is_test_only_cfg(lines[i]) {
            i += 1;
            continue;
        }
        let kind = region_kind(&lines, i);
        let mut braces = 0i32;
        let mut nest = 0i32;
        let mut entered = false;
        let mut outcome = None;
        for (k, line) in lines.iter().enumerate().skip(i) {
            braces += count(line, &['{']);
            if braces > 0 {
                entered = true;
            }
            braces -= count(line, &['}']);
            if entered {
                if braces <= 0 {
                    outcome = Some((k, RegionEnd::BodyClosed));
                    break;
                }
                continue;
            }
            nest += count(line, &['(', '[']) - count(line, &[')', ']']);
            if braces < 0 || nest < 0 {
                // The line closes the enclosing scope. It belongs to the region
                // only if the gated code shares it: `Variant }`, not `}`.
                let shares = !line.trim_start().starts_with(['}', ')', ']']);
                let last = if shares || k == i { k } else { k - 1 };
                outcome = Some((last, RegionEnd::EnclosingClose));
                break;
            }
            let code = line.trim_end();
            if nest == 0 && code.ends_with(';') {
                outcome = Some((k, RegionEnd::Semicolon));
                break;
            }
            if nest == 0 && kind == RegionKind::Member && code.ends_with(',') {
                outcome = Some((k, RegionEnd::Comma));
                break;
            }
        }
        let (last, ended_by) = outcome.unwrap_or((lines.len() - 1, RegionEnd::EndOfFile));
        regions.push(TestRegion {
            start: i + 1,
            end: last + 1,
            kind,
            ended_by,
        });
        i = last + 1;
    }
    regions
}

/// 1-based line numbers inside a test-only region, from STRIPPED text.
pub fn test_module_lines(stripped: &str) -> BTreeSet<usize> {
    test_regions(stripped)
        .iter()
        .flat_map(|r| r.start..=r.end)
        .collect()
}

/// The regions `test_regions` reads in `stripped` that cover code outside the
/// scope their gate sits in. See `containment_failures`.
pub fn region_containment_failures(stripped: &str) -> Vec<String> {
    containment_failures(stripped, &test_regions(stripped))
}

/// Which of `regions` cover code outside the scope their gate sits in, judged
/// by an independent reading of the braces: one pass over the whole file, not
/// the region reader's own local count.
///
/// A gate can only cover code inside the scope it is written in. So a region
/// that runs past the `}` closing that scope has taken production code with it,
/// and a region that runs to the end of the file never found its end. Each
/// failure is described in one line, for the report.
pub fn containment_failures(stripped: &str, regions: &[TestRegion]) -> Vec<String> {
    let lines: Vec<&str> = stripped.lines().collect();
    // Brace depth at the start of each line, and the lowest depth reached
    // anywhere on it.
    let mut depth_at_start = Vec::with_capacity(lines.len());
    let mut lowest_on_line = Vec::with_capacity(lines.len());
    let mut depth = 0i32;
    for line in &lines {
        depth_at_start.push(depth);
        let mut lowest = depth;
        for c in line.chars() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    lowest = lowest.min(depth);
                }
                _ => {}
            }
        }
        lowest_on_line.push(lowest);
    }

    let mut failures = Vec::new();
    for region in regions {
        let attr = region.start - 1;
        let scope = depth_at_start[attr];
        let scope_closes = (attr..lines.len()).find(|&k| lowest_on_line[k] < scope);
        if region.ended_by == RegionEnd::EndOfFile {
            failures.push(format!(
                "the {:?} gated on line {} runs to the end of the file",
                region.kind, region.start
            ));
        } else if let Some(close) = scope_closes {
            if region.end > close + 1 {
                failures.push(format!(
                    "the {:?} gated on line {} runs to line {}, past the `}}` on line {} that \
                     closes the scope the gate is written in ({:?})",
                    region.kind,
                    region.start,
                    region.end,
                    close + 1,
                    region.ended_by
                ));
            }
        }
    }
    failures
}
