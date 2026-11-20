// The shipped-profile guard (1.94.6 security review, H2).
//
// `cfg(debug_assertions)` is how this crate keeps development-only exceptions out of the
// shipped daemon: the development Electron parent that `caller::admitted` accepts, and the
// `privacy-test-auth` seam that `lib.rs` refuses with `compile_error!` outside a debug build.
// Both are sound only while no build of a shipped binary turns debug assertions on, and nothing
// else checks that. So this reads every place that could, and fails on any that does:
//
// * the workspace `Cargo.toml` and `.cargo/config.toml`: a `debug-assertions` setting other
//   than `false` in a shipped profile (`release`, `release-dist`, and every profile that
//   inherits from either, package overrides included), and a shipped profile whose inherited
//   default comes from `dev` or `test`, where debug assertions are on;
// * `scripts/**/*.sh`, the `Justfile`, the `Dockerfile` and `.github/workflows/*`: a
//   `CARGO_PROFILE_<shipped>_DEBUG_ASSERTIONS`, a `--config profile.<shipped>.debug-assertions`,
//   and any rustc flag that names debug assertions (`-C debug-assertions`,
//   `--cfg debug_assertions`), unless the setting says off.
//
// Comment lines are skipped. A form the guard cannot read (a whole profile written as an inline
// table) fails rather than passing. Lives in `browser_key`'s tests because that module holds
// the exception the guard exists for, and a Rust test runs in CI with no workflow change.

use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::LazyLock;

/// A mention of debug assertions, with the profile it names when it names one
/// (`CARGO_PROFILE_<P>_DEBUG_ASSERTIONS`, `profile.<p>.debug-assertions`).
static MENTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:CARGO_PROFILE_([A-Z0-9_]+?)_|profile\.([A-Za-z0-9_-]+)\.(?:[A-Za-z0-9_*.\x22-]+\.)?)?debug[-_]assertions",
    )
    .unwrap()
});

/// What follows a mention that turns debug assertions off.
static OFF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)^["']?\s*[=:]\s*["']?(?:false|off|no|n|0)\b"#).unwrap());

/// The profiles release builds use: `scripts/release.sh`, the `Justfile` and the `Dockerfile`
/// build with `--release`, and `release-dist` is the declared distribution profile.
const SHIPPED_ROOTS: &[&str] = &["release", "release-dist"];

/// Cargo's built-in profiles, and whether each turns debug assertions on by default.
fn builtin_default(profile: &str) -> Option<bool> {
    match profile {
        "dev" | "test" => Some(true),
        "release" | "bench" => Some(false),
        _ => None,
    }
}

/// A profile's name as Cargo compares it across TOML (`release-dist`) and the environment
/// (`CARGO_PROFILE_RELEASE_DIST_*`).
fn profile_key(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace('-', "_")
}

/// `line` up to its first `#` outside a string.
fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    for (at, c) in line.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '#') => return line.get(..at).unwrap_or(line),
            _ => {}
        }
    }
    line
}

/// The byte offset of the first `wanted` outside a string.
fn find_unquoted(text: &str, wanted: char) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (at, c) in text.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, c) if c == wanted => return Some(at),
            _ => {}
        }
    }
    None
}

/// A dotted TOML key or table name, split outside quotes, each part trimmed and unquoted.
fn split_dotted(path: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut rest = path;
    while let Some(at) = find_unquoted(rest, '.') {
        let (part, after) = rest.split_at(at);
        parts.push(part);
        rest = after.get(1..).unwrap_or_default();
    }
    parts.push(rest);
    parts
        .into_iter()
        .map(|part| part.trim().trim_matches('"').trim_matches('\'').to_string())
        .collect()
}

/// One non-blank, comment-free TOML line, read just far enough for this guard: the dotted
/// path it assigns (its table's path plus its key) or, for a header or a continuation line,
/// its table's path.
struct TomlLine<'a> {
    number: usize,
    text: &'a str,
    path: Vec<String>,
    value: Option<&'a str>,
}

fn toml_lines(contents: &str) -> Vec<TomlLine<'_>> {
    let mut table: Vec<String> = Vec::new();
    let mut lines = Vec::new();
    for (index, raw) in contents.lines().enumerate() {
        let text = strip_comment(raw).trim();
        if text.is_empty() {
            continue;
        }
        if text.starts_with('[') && !text.starts_with("[\"") {
            let inner = text.trim_start_matches('[');
            if let Some(end) = find_unquoted(inner, ']') {
                table = split_dotted(inner.get(..end).unwrap_or_default());
                lines.push(TomlLine {
                    number: index + 1,
                    text,
                    path: table.clone(),
                    value: None,
                });
                continue;
            }
        }
        match find_unquoted(text, '=') {
            Some(at) => {
                let mut path = table.clone();
                let (key, value) = text.split_at(at);
                path.extend(split_dotted(key));
                lines.push(TomlLine {
                    number: index + 1,
                    text,
                    path,
                    value: Some(value.get(1..).unwrap_or_default().trim()),
                });
            }
            None => lines.push(TomlLine {
                number: index + 1,
                text,
                path: table.clone(),
                value: None,
            }),
        }
    }
    lines
}

fn is_debug_assertions_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "debug-assertions" | "debug_assertions"
    )
}

/// What the workspace's TOML says about each named profile.
#[derive(Default)]
struct Profiles {
    /// profile -> the profile it inherits from (both as [`profile_key`]).
    inherits: BTreeMap<String, String>,
    /// profile -> its own top-level `debug-assertions`; anything but `false` reads as on.
    debug_assertions: BTreeMap<String, bool>,
}

fn read_profiles(file: &str, lines: &[TomlLine<'_>], profiles: &mut Profiles) -> Vec<String> {
    let mut violations = Vec::new();
    for line in lines {
        let Some(value) = line.value else { continue };
        if line.path.first().map(String::as_str) != Some("profile") {
            continue;
        }
        if line.path.len() <= 2 && value.starts_with('{') {
            violations.push(format!(
                "{file}:{}: `{}` writes a profile as an inline table, which this guard cannot \
                 read; write it as a `[profile.<name>]` table",
                line.number, line.text
            ));
            continue;
        }
        if line.path.len() != 3 {
            continue;
        }
        let profile = profile_key(&line.path[1]);
        if line.path[2] == "inherits" {
            let parent = value.trim_matches('"').trim_matches('\'');
            profiles.inherits.insert(profile, profile_key(parent));
        } else if is_debug_assertions_key(&line.path[2]) {
            profiles
                .debug_assertions
                .insert(profile, value.trim() != "false");
        }
    }
    violations
}

/// Every profile a shipped build reads a setting from: each shipped root, each profile that
/// inherits from one, and every profile on their inheritance chains. Also the violations: a
/// shipped profile whose effective `debug-assertions` is on, or whose chain cannot be followed.
fn shipped_chains(profiles: &Profiles) -> (BTreeSet<String>, Vec<String>) {
    let defined: BTreeSet<String> = profiles
        .inherits
        .keys()
        .chain(profiles.debug_assertions.keys())
        .cloned()
        .chain(SHIPPED_ROOTS.iter().map(|root| profile_key(root)))
        .collect();
    let roots: BTreeSet<String> = SHIPPED_ROOTS.iter().map(|root| profile_key(root)).collect();
    let chain_of = |start: &str| -> Result<Vec<String>, String> {
        let mut chain = vec![start.to_string()];
        loop {
            let current = chain.last().unwrap().clone();
            if builtin_default(&current.replace('_', "-")).is_some() {
                return Ok(chain);
            }
            let Some(parent) = profiles.inherits.get(&current) else {
                return Err(if current == start {
                    format!("profile `{start}` is not built in and inherits from no profile")
                } else {
                    format!("profile `{start}` inherits from `{current}`, which is not defined")
                });
            };
            if chain.contains(parent) {
                return Err(format!("profile `{start}` inherits in a cycle"));
            }
            chain.push(parent.clone());
        }
    };

    let mut shipped = BTreeSet::new();
    let mut violations = Vec::new();
    for profile in &defined {
        let chain = match chain_of(profile) {
            Ok(chain) => chain,
            Err(problem) => {
                if roots.contains(profile) {
                    violations.push(problem);
                }
                continue;
            }
        };
        if !chain.iter().any(|link| roots.contains(link)) {
            continue;
        }
        shipped.extend(chain.iter().cloned());
        let effective = chain
            .iter()
            .find_map(|link| profiles.debug_assertions.get(link).copied())
            .or_else(|| builtin_default(&chain.last().unwrap().replace('_', "-")))
            .unwrap_or(true);
        if effective {
            violations.push(format!(
                "shipped profile `{profile}` builds with debug assertions on (inheritance: {})",
                chain.join(" -> ")
            ));
        }
    }
    (shipped, violations)
}

/// Each mention of debug assertions on `line`, judged: off, or in a profile that does not ship,
/// is fine; anything else is a violation. `toml_profile` is the profile a TOML assignment sits
/// in, for a mention that names none itself.
fn judge_line(
    file: &str,
    number: usize,
    line: &str,
    toml_profile: Option<&str>,
    shipped: &BTreeSet<String>,
) -> Option<String> {
    for found in MENTION.captures_iter(line) {
        let whole = found.get(0).unwrap();
        if OFF.is_match(line.get(whole.end()..).unwrap_or_default()) {
            continue;
        }
        let named = found
            .get(1)
            .or_else(|| found.get(2))
            .map(|profile| profile.as_str())
            .or(toml_profile);
        if named.is_some_and(|profile| !shipped.contains(&profile_key(profile))) {
            continue;
        }
        return Some(format!(
            "{file}:{number}: `{}` turns debug assertions on for a shipped build",
            line.trim()
        ));
    }
    None
}

/// The whole check over file contents: `toml` is the workspace manifest and Cargo's config,
/// `text` every script, Justfile, Dockerfile and workflow a release build runs through.
fn shipped_profile_violations(toml: &[(&str, &str)], text: &[(&str, &str)]) -> Vec<String> {
    let parsed: Vec<(&str, Vec<TomlLine<'_>>)> = toml
        .iter()
        .map(|(file, contents)| (*file, toml_lines(contents)))
        .collect();
    let mut profiles = Profiles::default();
    let mut violations = Vec::new();
    for (file, lines) in &parsed {
        violations.extend(read_profiles(file, lines, &mut profiles));
    }
    let (shipped, chain_violations) = shipped_chains(&profiles);
    violations.extend(chain_violations);
    for (file, lines) in &parsed {
        for line in lines {
            let profile = (line.path.first().map(String::as_str) == Some("profile"))
                .then(|| line.path.get(1).map(String::as_str))
                .flatten();
            violations.extend(judge_line(file, line.number, line.text, profile, &shipped));
        }
    }
    for (file, contents) in text {
        for (index, raw) in contents.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            violations.extend(judge_line(file, index + 1, line, None, &shipped));
        }
    }
    violations
}

fn read(root: &Path, relative: &str) -> String {
    std::fs::read_to_string(root.join(relative)).unwrap_or_else(|error| {
        panic!(
            "{relative}: {error}. The shipped-profile guard reads it; if it moved, point the \
             guard at its new home rather than dropping it"
        )
    })
}

#[test]
fn no_shipped_build_turns_debug_assertions_on() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut toml = vec![("Cargo.toml".to_string(), read(&root, "Cargo.toml"))];
    for config in [".cargo/config.toml", ".cargo/config"] {
        if root.join(config).is_file() {
            toml.push((config.to_string(), read(&root, config)));
        }
    }
    let mut text = Vec::new();
    for required in ["scripts/release.sh", "Justfile", "Dockerfile"] {
        text.push((required.to_string(), read(&root, required)));
    }
    for entry in walkdir::WalkDir::new(root.join("scripts"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "sh") && !path.ends_with("release.sh") {
            let relative = path.strip_prefix(&root).unwrap().display().to_string();
            text.push((relative.clone(), read(&root, &relative)));
        }
    }
    let mut workflows = 0;
    for entry in std::fs::read_dir(root.join(".github/workflows")).expect(".github/workflows") {
        let path = entry.unwrap().path();
        if path
            .extension()
            .is_some_and(|ext| ext == "yml" || ext == "yaml")
        {
            let relative = path.strip_prefix(&root).unwrap().display().to_string();
            text.push((relative.clone(), read(&root, &relative)));
            workflows += 1;
        }
    }
    assert!(workflows > 0, "no workflow was read");

    let toml: Vec<(&str, &str)> = toml.iter().map(|(f, c)| (f.as_str(), c.as_str())).collect();
    let text: Vec<(&str, &str)> = text.iter().map(|(f, c)| (f.as_str(), c.as_str())).collect();
    let violations = shipped_profile_violations(&toml, &text);
    assert!(
        violations.is_empty(),
        "a shipped biorouterd would compile its cfg(debug_assertions) exceptions (the \
         development Electron parent in browser_key.rs, the privacy-test-auth seam):\n{}",
        violations.join("\n")
    );
}

/// Each spelling the guard exists for fails, and each harmless neighbour passes, so the
/// real-tree test above cannot pass by reading nothing.
#[test]
fn the_guard_refuses_each_way_to_turn_debug_assertions_on() {
    let manifest = "[profile.release]\nstrip = true\n\n[profile.release-dist]\n\
                    inherits = \"release\"\nlto = \"thin\"\n\n[profile.quick]\n\
                    inherits = \"release\"\nopt-level = 1\n";
    let check = |extra_toml: &str, text: &str| {
        let toml = format!("{manifest}\n{extra_toml}");
        shipped_profile_violations(&[("Cargo.toml", &toml)], &[("ci.yml", text)])
    };
    assert_eq!(
        check("", ""),
        Vec::<String>::new(),
        "the shipped manifest's shape"
    );

    for (extra_toml, text) in [
        ("[profile.release]\ndebug-assertions = true\n", ""),
        ("[profile.release-dist]\ndebug-assertions = true\n", ""),
        ("[profile.quick]\ndebug_assertions = true\n", ""),
        (
            "[profile.release.package.biorouter]\ndebug-assertions = true\n",
            "",
        ),
        (
            "[profile.release.package.\"*\"]\ndebug-assertions = true\n",
            "",
        ),
        ("[profile]\nrelease.debug-assertions = true\n", ""),
        ("profile.release-dist.debug-assertions = true\n", ""),
        (
            "[profile]\nrelease-dist = { debug-assertions = false }\n",
            "",
        ),
        (
            "[profile.dist]\ninherits = \"release-dist\"\ndebug-assertions = true\n",
            "",
        ),
        ("[profile.release]\ndebug-assertions = \"yes\"\n", ""),
        (
            "[env]\nCARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS = \"true\"\n",
            "",
        ),
        ("[build]\nrustflags = [\"-C\", \"debug-assertions\"]\n", ""),
        (
            "",
            "  env:\n    CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS: true\n",
        ),
        (
            "",
            "export CARGO_PROFILE_RELEASE_DIST_DEBUG_ASSERTIONS=true\n",
        ),
        (
            "",
            "RUSTFLAGS=\"-C debug-assertions\" cargo build --release\n",
        ),
        (
            "",
            "RUSTFLAGS='-Cdebug-assertions=on' cargo build --release\n",
        ),
        (
            "",
            "RUSTFLAGS='--cfg debug_assertions' cargo build --release\n",
        ),
        (
            "",
            "cargo build --release --config profile.release.debug-assertions=true\n",
        ),
        (
            "",
            "RUSTFLAGS='-C debug-assertions=off -C debug-assertions' cargo build\n",
        ),
    ] {
        assert!(
            !check(extra_toml, text).is_empty(),
            "not refused: {extra_toml:?} {text:?}"
        );
    }
    // A shipped profile that inherits dev's default (debug assertions on).
    let from_dev = "[profile.release-dist]\ninherits = \"dev\"\n";
    assert!(!shipped_profile_violations(&[("Cargo.toml", from_dev)], &[]).is_empty());
    let undefined = "[profile.release-dist]\ninherits = \"nowhere\"\n";
    assert!(!shipped_profile_violations(&[("Cargo.toml", undefined)], &[]).is_empty());

    for (extra_toml, text) in [
        ("[profile.dev]\ndebug-assertions = true\n", ""),
        ("[profile.dev.package.\"*\"]\ndebug-assertions = true\n", ""),
        ("[profile.release]\ndebug-assertions = false\n", ""),
        (
            "[profile.release-dist]\ninherits = \"release\"\ndebug-assertions = false\n",
            "",
        ),
        (
            "[profile.ci]\ninherits = \"dev\"\ndebug-assertions = true\n",
            "",
        ),
        ("# [profile.release] debug-assertions = true\n", ""),
        (
            "",
            "# RUSTFLAGS=\"-C debug-assertions\" is never set for a release\n",
        ),
        ("", "  CARGO_PROFILE_DEV_DEBUG_ASSERTIONS: true\n"),
        ("", "export CARGO_PROFILE_TEST_DEBUG_ASSERTIONS=true\n"),
        (
            "",
            "RUSTFLAGS='-C debug-assertions=off' cargo build --release\n",
        ),
        ("", "export CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=false\n"),
        (
            "",
            "cargo test --config profile.dev.debug-assertions=true\n",
        ),
        ("", "export CARGO_PROFILE_RELEASE_DEBUG=0\n"),
    ] {
        let found = check(extra_toml, text);
        assert!(found.is_empty(), "{extra_toml:?} {text:?}: {found:?}");
    }
    // `release-dist` with an explicit `false` may inherit dev's other settings.
    let off_from_dev = "[profile.release-dist]\ninherits = \"dev\"\ndebug-assertions = false\n";
    assert_eq!(
        shipped_profile_violations(&[("Cargo.toml", off_from_dev)], &[]),
        Vec::<String>::new()
    );
}
