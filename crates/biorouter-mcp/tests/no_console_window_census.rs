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
//! it, mentions one of the preparation helpers in [`COVERING_CALLS`].
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
const EXEMPT: &[(&str, &str)] = &[
    (
        "crates/biorouter-mcp/src/computercontroller/platform/macos.rs",
        "whole file is #[cfg(target_os = \"macos\")] — osascript does not exist on Windows",
    ),
    (
        "crates/biorouter-mcp/src/computercontroller/platform/linux.rs",
        "`mod linux;` is #[cfg(target_os = \"linux\")] in platform/mod.rs, so this          file is never compiled for Windows — xdotool/xclip/wmctrl are X11 and          Wayland tools",
    ),
    (
        "crates/biorouter-sandbox/src/shell_sandbox/linux.rs",
        "whole file is Linux-only (seccomp/landlock); never compiled for Windows",
    ),
    (
        "crates/biorouter/src/privacy/system_auth_macos.rs",
        "macOS authorization UI only",
    ),
    (
        "crates/biorouter/src/privacy/system_auth_polkit.rs",
        "polkit is Linux-only",
    ),
    (
        "crates/biorouter/src/agents/bug_report/issue.rs",
        "`gh` runs only from the maintainer bug-report flow, never on a user turn",
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
];

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is <root>/crates/biorouter-mcp
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<crate>/ has two ancestors")
        .to_path_buf()
}

fn rust_sources(root: &Path) -> Vec<PathBuf> {
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

/// Blank out comments, string literals and char literals, keeping every byte's
/// line position. Braces inside them must not be counted.
///
/// ⚠ Both failure modes below were observed in this tree, not imagined.
/// `agent_drafter/render.rs` *generates* shell and JavaScript, so its raw string
/// literals are full of unbalanced `{` and `}`; counting them ended its test
/// module hundreds of lines early and reported test code as production. The
/// privacy census (`crates/biorouter/tests/privacy_guard_wiring.rs`) records the
/// same lesson from the other direction — a gate that passed on a mention in a
/// comment.
fn strip_literals(text: &str) -> String {
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
                let close: String = std::iter::once('"').chain(std::iter::repeat_n('#', hashes)).collect();
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
        // Char literal — `'{'` would otherwise unbalance the count. Lifetimes
        // (`'a`) have no closing quote, so require one within three chars.
        if c == '\'' {
            let close = (1..=3).find(|&k| i + k < bytes.len() && bytes[i + k] == '\'');
            if let Some(k) = close {
                for _ in 0..=k {
                    out.push(' ');
                }
                i += k + 1;
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

/// Split a `cfg` predicate list on the commas that sit at nesting depth zero,
/// so `all(test, windows)` yields `["test", "windows"]` and
/// `all(test, any(a, b))` yields `["test", "any(a, b)"]` rather than splitting
/// the inner list.
fn split_top_level(list: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in list.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(list[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(list[start..].trim());
    parts
}

/// Whether a `cfg` predicate holds ONLY in a test build.
///
/// `all(...)` is test-only when any of its arms is, because every arm must
/// hold. `any(...)` and `not(...)` are deliberately NOT test-only: an item
/// gated that way also exists in a normal build, so its spawn sites are
/// production code and must stay in the census. Erring that way keeps this
/// function's mistakes in the direction of over-reporting, which is visible,
/// rather than under-reporting, which is silent.
fn cfg_is_test_only(predicate: &str) -> bool {
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

/// Whether an attribute line gates the item below it to test builds only.
///
/// ⚠ Matching the literal string `#[cfg(test)]` is not enough, and it fails
/// SILENTLY in the dangerous direction. A module gated `#[cfg(all(test,
/// windows))]` is invisible to such a matcher, so every spawn site inside it
/// is reported as production code — which is exactly what happened when the
/// Claude Code shim's Windows-only test module landed: a census written
/// against one spelling met a second one and cried wolf. There are 14 such
/// modules in this repo, so the spelling is ordinary, not exotic.
fn is_test_only_cfg(line: &str) -> bool {
    line.trim_start()
        .strip_prefix("#[cfg(")
        .and_then(|rest| rest.strip_suffix(")]"))
        .is_some_and(cfg_is_test_only)
}

/// Line numbers inside a test-only module. Test children are spawned by a
/// console test runner and never open a window.
///
/// ⚠ The obvious brace-matcher is wrong, and wrong in the direction that makes
/// this census under-report. A first draft broke out of the module the moment
/// `depth` returned to zero on a line containing `}` — which any single-line
/// `fn f() {}` satisfies — so it ended a long test module at its first short
/// function and reported the rest as production code. Enter the module on the
/// first `{`, and only then let a return to zero close it.
fn test_module_lines(text: &str) -> BTreeSet<usize> {
    let stripped = strip_literals(text);
    let lines: Vec<&str> = stripped.lines().collect();
    let mut marked = BTreeSet::new();
    let mut i = 0;
    while i < lines.len() {
        if !is_test_only_cfg(lines[i]) {
            i += 1;
            continue;
        }
        let mut depth = 0i32;
        let mut entered = false;
        let mut k = i;
        while k < lines.len() {
            marked.insert(k + 1);
            depth += lines[k].matches('{').count() as i32;
            if depth > 0 {
                entered = true;
            }
            depth -= lines[k].matches('}').count() as i32;
            if entered && depth <= 0 {
                break;
            }
            k += 1;
        }
        i = k + 1;
    }
    marked
}

/// Whether the nearest `cfg` gate above `line` excludes Windows.
///
/// This is the honest reason most remaining sites need no flag: a `ps`, an
/// `xdotool` or an `osascript` branch is never compiled for Windows, so it can
/// no more flash a console than it can run. Reading the gate beats listing the
/// lines, because the gate moves with the code.
fn gated_away_from_windows(lines: &[&str], line: usize) -> bool {
    const NOT_WINDOWS: &[&str] = &[
        "#[cfg(unix)]",
        "#[cfg(not(windows))]",
        "#[cfg(target_os = \"linux\")]",
        "#[cfg(target_os = \"macos\")]",
        "#[cfg(not(target_os = \"windows\"))]",
        "#[cfg(any(target_os = \"linux\", target_os = \"macos\"))]",
    ];
    let is_gate = |l: &str| NOT_WINDOWS.iter().any(|g| l.trim_start().starts_with(g));

    // ⚠ Only a gate that governs THIS statement counts. A first draft scanned
    // back ten lines and exempted a spawn because an unrelated
    // `#[cfg(not(windows))] let node = …` sat above it — the gate belonged to a
    // different binding, and the spawn ran on Windows regardless. So: the line
    // immediately above, or the attribute on the enclosing `fn`.
    let mut n = line.saturating_sub(1); // 0-based index of the previous line
    while n > 0 {
        let prev = lines[n - 1].trim();
        if prev.is_empty() || prev.starts_with("//") {
            n -= 1;
            continue;
        }
        if is_gate(prev) {
            return true;
        }
        break;
    }
    // The enclosing fn's own attribute.
    let fn_line = (0..line.saturating_sub(1)).rev().find(|&k| {
        let l = lines[k];
        l.starts_with("fn ") || l.starts_with("pub fn ") || l.starts_with("async fn ")
    });
    if let Some(f) = fn_line {
        for back in 1..=3 {
            if f >= back && is_gate(lines[f - back]) {
                return true;
            }
        }
    }
    false
}

/// The text of the item containing `line`, approximated as the span back to the
/// previous top-level `fn` and forward to the next one. Coarse on purpose — see
/// the module header.
fn enclosing_item(lines: &[&str], line: usize) -> String {
    let idx = line - 1;
    let start = (0..idx).rev().find(|&n| is_fn_start(lines[n])).unwrap_or(0);
    let end = (idx + 1..lines.len())
        .find(|&n| is_fn_start(lines[n]))
        .unwrap_or(lines.len());
    lines[start..end].join("\n")
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
                Some(close) => scoped[close + 1..].trim_start(),
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
        // `extern "C" fn`
        if rest.starts_with('"') {
            if let Some(close) = rest[1..].find('"') {
                rest = rest[close + 2..].trim_start();
            }
        }
        if rest == before {
            break;
        }
    }
    rest.starts_with("fn ")
}

struct Site {
    file: String,
    line: usize,
    code: String,
}

fn uncovered_sites() -> (Vec<Site>, usize, usize) {
    let root = repo_root();
    let files = rust_sources(&root);
    let mut uncovered = Vec::new();
    let mut total_sites = 0usize;

    for path in &files {
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        if !text.contains("Command::new") {
            continue;
        }
        let in_test = test_module_lines(&text);
        let lines: Vec<&str> = text.lines().collect();

        for (idx, raw) in lines.iter().enumerate() {
            let line = idx + 1;
            let trimmed = raw.trim_start();
            if !raw.contains("Command::new") || trimmed.starts_with("//") {
                continue;
            }
            total_sites += 1;
            if in_test.contains(&line) {
                continue;
            }
            if EXEMPT.iter().any(|(p, _)| rel.contains(p)) {
                continue;
            }
            if gated_away_from_windows(&lines, line) {
                continue;
            }
            let item = enclosing_item(&lines, line);
            if COVERING_CALLS.iter().any(|c| item.contains(c)) {
                continue;
            }
            uncovered.push(Site {
                file: rel.clone(),
                line,
                code: trimmed.chars().take(80).collect(),
            });
        }
    }
    (uncovered, files.len(), total_sites)
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
        // Computer Controller drives the desktop through PowerShell.
        (
            "crates/biorouter-mcp/src/computercontroller/mod.rs",
            "no_console_window",
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
        "#[cfg(not(test))]",                 // the exact inverse
        "#[cfg(any(test, windows))]",        // holds on windows WITHOUT test
        "#[cfg(all(not(test), windows))]",   // holds only outside test
        "#[cfg(feature = \"test-utils\")]",  // merely contains the word "test"
        "#[cfg(feature = \"integration-test\")]",
        "let x = 1;",                        // not an attribute at all
        "#[test]",                           // a test fn, not a module gate
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

    let marked = test_module_lines(&text);
    assert!(
        marked.contains(&(spawn_at + 1)),
        "line {} of {rel} is inside `{gate}` and must be excluded from the census",
        spawn_at + 1
    );
}
