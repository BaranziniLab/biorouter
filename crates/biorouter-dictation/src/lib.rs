//! Biorouter's local dictation engine (`docs/dictation/engine-and-models.md`).
//!
//! whisper.cpp, vendored under `vendor/whisper.cpp`, is compiled into every shipped
//! `biorouter` and `biorouterd` (DD-2, DD-5) and driven through the C++ shim in `shim/`.
//! It only ever runs in a worker process ([`worker`]), never inside the daemon or an
//! interactive CLI (DD-4). The CPU gate ([`cpu`]) runs before any worker starts.
//!
//! The parent half is plain Rust and builds on every target, with or without the
//! engine: the frame protocol ([`protocol`]), the PCM helpers and the segment planner
//! ([`audio`]) and the [`supervisor`] that spawns, times and restarts the worker. The
//! speech model catalog ([`catalog`]), the installer ([`models`]) and the suitability
//! rules ([`hardware`]) build everywhere too, and so does [`recognize`], which decides
//! what of the engine's output is speech. So does the personal dictionary's [`store`],
//! which the daemon, the CLI and the agent's capability share under one lock.
//!
//! A leaf crate: no Biorouter dependency, and every path is an argument.

pub mod audio;
pub mod build_info;
pub mod catalog;
pub mod cpu;
#[cfg(all(feature = "engine", not(br_no_native)))]
pub mod engine;
pub mod hardware;
pub mod models;
pub mod protocol;
pub mod recognize;
pub mod store;
pub mod supervisor;
/// The worker's marker, roles and exit codes. Present in every build, so both `main()`
/// functions can call `worker::run_if_invoked()` and the supervisor can read the exit
/// codes; without a linked engine every role answers 2.
pub mod worker;

/// True only when whisper.cpp is linked into this binary.
pub const ENGINE_COMPILED: bool = cfg!(all(feature = "engine", not(br_no_native)));

#[cfg(test)]
mod tests {
    use std::path::Path;

    fn sibling_source(relative: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(relative);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{} is not readable: {e}", path.display()))
    }

    /// DD-4: a worker starts before the runtime, logging, configuration, the shell
    /// sandbox helper and the legacy folder move. In both `main()` functions the statement
    /// right after `complete_libnotify_init()` is the cfg'd `run_if_invoked()`, and it
    /// comes before `run_on_agent_stack`.
    #[test]
    fn worker_entry_runs_first() {
        for file in ["biorouter-server/src/main.rs", "biorouter-cli/src/main.rs"] {
            let source = sibling_source(file);
            let main_at = source
                .find("\nfn main()")
                .unwrap_or_else(|| panic!("{file} has no fn main()"));
            let body = source.split_at(main_at).1;
            let statements: Vec<&str> = body
                .lines()
                .skip(1)
                .map(|line| line.split("//").next().unwrap_or_default().trim())
                .filter(|line| !line.is_empty())
                .collect();
            let first = statements
                .iter()
                .position(|s| *s == "biorouter::fork_safety::complete_libnotify_init();")
                .unwrap_or_else(|| panic!("{file}: complete_libnotify_init() is not in main"));
            assert_eq!(
                statements.get(first + 1..first + 3),
                Some(
                    [
                        "#[cfg(feature = \"local-dictation\")]",
                        "biorouter_dictation::worker::run_if_invoked();",
                    ]
                    .as_slice()
                ),
                "{file}: the worker entry must directly follow complete_libnotify_init()"
            );
            let stack = statements
                .iter()
                .position(|s| s.contains("run_on_agent_stack"))
                .unwrap_or_else(|| panic!("{file}: run_on_agent_stack is not in main"));
            assert!(
                stack > first + 2,
                "{file}: the worker entry must precede run_on_agent_stack"
            );
        }
    }

    /// DD-10 rule 6 and DD-11, read from this test executable, which is linked like the
    /// shipped binaries but not stripped: the minimum macOS is 12.0, every Metal class
    /// newer than that is a weak import, and BLAS's `$NEWLAPACK` binding (macOS 13.3) is
    /// absent.
    #[cfg(target_os = "macos")]
    #[test]
    fn this_binary_starts_on_macos_12() {
        let exe = std::env::current_exe().expect("the test executable");
        let load_commands = tool_output("otool", &["-l"], &exe);
        let mut minos = None;
        let mut in_build_version = false;
        for line in load_commands.lines().map(str::trim) {
            if line.starts_with("cmd ") {
                in_build_version = line == "cmd LC_BUILD_VERSION";
            } else if in_build_version && line.starts_with("minos ") {
                minos = line.split_whitespace().nth(1).map(str::to_string);
            }
        }
        assert_eq!(minos.as_deref(), Some("12.0"), "LC_BUILD_VERSION minos");

        let symbols = tool_output("nm", &["-m"], &exe);
        assert!(
            !symbols.contains("$NEWLAPACK"),
            "BLAS's $NEWLAPACK binding is linked"
        );
        if crate::ENGINE_COMPILED && cfg!(target_arch = "aarch64") {
            let residency: Vec<&str> = symbols
                .lines()
                .filter(|line| line.contains("_OBJC_CLASS_$_MTLResidencySetDescriptor"))
                .collect();
            assert!(
                !residency.is_empty(),
                "the Metal backend was not linked into this test executable"
            );
            assert!(
                residency.iter().all(|line| line.contains(" weak ")),
                "MTLResidencySetDescriptor (macOS 15) must be a weak import: {residency:?}"
            );
        }
    }

    /// The end (one past) of the comment, string or character literal that starts at
    /// `i`, if one starts there. Raw strings with any number of `#`, escapes, and nested
    /// block comments are followed; a lifetime (`'a`) is not a literal.
    fn literal_end(s: &[char], i: usize) -> Option<usize> {
        let at = |k: usize| s.get(k).copied();
        match (at(i)?, at(i + 1)) {
            ('/', Some('/')) => Some((i..s.len()).find(|&k| s[k] == '\n').unwrap_or(s.len())),
            ('/', Some('*')) => {
                let (mut depth, mut k) = (0usize, i);
                while k < s.len() {
                    match (s[k], at(k + 1)) {
                        ('/', Some('*')) => (depth, k) = (depth + 1, k + 2),
                        ('*', Some('/')) => {
                            depth -= 1;
                            k += 2;
                            if depth == 0 {
                                return Some(k);
                            }
                        }
                        _ => k += 1,
                    }
                }
                Some(s.len())
            }
            ('"', _) => {
                let mut k = i + 1;
                while k < s.len() {
                    match s[k] {
                        '\\' => k += 2,
                        '"' => return Some(k + 1),
                        _ => k += 1,
                    }
                }
                Some(s.len())
            }
            ('r', Some('"' | '#')) => {
                let hashes = s[i + 1..].iter().take_while(|&&c| c == '#').count();
                if at(i + 1 + hashes) != Some('"') {
                    return None; // a raw identifier such as r#type
                }
                let closing: Vec<char> = std::iter::once('"')
                    .chain(std::iter::repeat_n('#', hashes))
                    .collect();
                let start = i + 2 + hashes;
                Some(
                    (start..s.len())
                        .find(|&k| s[k..].starts_with(&closing))
                        .map_or(s.len(), |k| k + closing.len()),
                )
            }
            // '\n', '\'', '\\', '\x7f', '\u{1F600}': the escaped character is at i + 2,
            // and no escape holds a quote after it.
            ('\'', Some('\\')) => (i + 3..s.len()).find(|&k| s[k] == '\'').map(|k| k + 1),
            ('\'', Some(_)) if at(i + 2) == Some('\'') => Some(i + 3),
            _ => None,
        }
    }

    /// The index one past the bracket that closes the one at `open`, literals skipped.
    fn closing(s: &[char], open: usize) -> usize {
        let (mut depth, mut k) = (0usize, open);
        while k < s.len() {
            if let Some(end) = literal_end(s, k) {
                k = end;
                continue;
            }
            match s[k] {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return k + 1;
                    }
                }
                _ => {}
            }
            k += 1;
        }
        s.len()
    }

    /// The end of the item that starts at `i`: its first `;` outside brackets, or the
    /// brace block it opens (a `mod`, a `fn`, an `impl`). `#[cfg(test)] mod x;` is one
    /// declaration, so the code after it is still scanned.
    fn item_end(s: &[char], i: usize) -> usize {
        let mut k = i;
        while k < s.len() {
            if let Some(end) = literal_end(s, k) {
                k = end;
                continue;
            }
            match s[k] {
                ';' => return k + 1,
                '{' => return closing(s, k),
                '(' | '[' => k = closing(s, k),
                _ => k += 1,
            }
        }
        s.len()
    }

    /// `source` with comments and every item behind `#[cfg(test)]` left out, whatever
    /// lines the item spans and whatever attributes sit between.
    fn production_code(source: &str) -> Vec<char> {
        const TEST_ONLY: &str = "#[cfg(test)]";
        let s: Vec<char> = source.chars().collect();
        let marker: Vec<char> = TEST_ONLY.chars().collect();
        let mut out = Vec::with_capacity(s.len());
        let mut k = 0;
        while k < s.len() {
            if s[k] == '/' && matches!(s.get(k + 1), Some('/' | '*')) {
                k = literal_end(&s, k).unwrap_or(k + 1);
                out.push(' ');
                continue;
            }
            if let Some(end) = literal_end(&s, k) {
                out.extend_from_slice(&s[k..end]);
                k = end;
                continue;
            }
            if s[k..].starts_with(&marker) {
                let mut j = k + marker.len();
                loop {
                    while j < s.len() && s[j].is_whitespace() {
                        j += 1;
                    }
                    if s.get(j) == Some(&'/') {
                        j = literal_end(&s, j).unwrap_or(j + 1);
                    } else if s.get(j) == Some(&'#') && s.get(j + 1) == Some(&'[') {
                        j = closing(&s, j + 1);
                    } else {
                        break;
                    }
                }
                k = item_end(&s, j);
                continue;
            }
            out.push(s[k]);
            k += 1;
        }
        out
    }

    /// The `include_bytes!` and `include_str!` arguments in the production code of
    /// `source` that name a model: a `.bin` file or anything under the vendored models
    /// folder. The argument runs to its closing bracket across any number of lines, so
    /// rustfmt's wrapped forms are read whole.
    fn embedded_models(source: &str) -> Vec<String> {
        let s = production_code(source);
        let mut found = Vec::new();
        let mut k = 0;
        while k < s.len() {
            if let Some(end) = literal_end(&s, k) {
                k = end;
                continue;
            }
            let named = ["include_bytes!", "include_str!"].into_iter().find(|name| {
                let name: Vec<char> = name.chars().collect();
                s[k..].starts_with(&name)
                    && !k
                        .checked_sub(1)
                        .is_some_and(|p| s[p].is_alphanumeric() || s[p] == '_')
            });
            let Some(name) = named else {
                k += 1;
                continue;
            };
            let mut open = k + name.len();
            while open < s.len() && s[open].is_whitespace() {
                open += 1;
            }
            if !matches!(s.get(open), Some('(' | '[' | '{')) {
                k += 1;
                continue;
            }
            let end = closing(&s, open);
            let argument: String = s[open + 1..end.saturating_sub(1).max(open + 1)]
                .iter()
                .collect();
            // A Windows path in a literal is written with escaped backslashes.
            let normalized = argument.replace("\\\\", "/").replace('\\', "/");
            if normalized.contains(".bin\"")
                || normalized.contains("vendor/whisper.cpp/source/models/")
            {
                found.push(argument.split_whitespace().collect::<Vec<_>>().join(" "));
            }
            k = end;
        }
        found
    }

    fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// R6: no speech model is compiled into a binary. The magic scans of the packaging
    /// guards cannot see a model inside a binary (the loader itself holds the magic), so
    /// this reads the sources of every crate that could embed one. The bench clip
    /// `samples/jfk.wav` is speech, not a model, and stays allowed.
    #[test]
    fn no_model_is_embedded() {
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut files = Vec::new();
        for dir in [
            "biorouter-dictation/src",
            "biorouter-dictation-capture/src",
            "biorouter/src/dictation",
        ] {
            rust_files(&crates.join(dir), &mut files);
        }
        assert!(
            files.len() >= 10,
            "the scan found only {} files: it has stopped looking",
            files.len()
        );
        let mut offenders = Vec::new();
        for file in &files {
            let source = std::fs::read_to_string(file).unwrap();
            for path in embedded_models(&source) {
                offenders.push(format!("{}: {path}", file.display()));
            }
        }
        assert!(offenders.is_empty(), "a model is embedded: {offenders:#?}");
    }

    /// The scan itself fails on what it exists to catch, in every shape rustfmt leaves.
    #[test]
    fn the_embedded_model_scan_catches_a_model() {
        let planted = [
            "const M: &[u8] = include_bytes!(\"../models/ggml-small.en-q8_0.bin\");",
            "static D: &[u8] = include_bytes!(concat!(env!(\"X\"), \"/vendor/whisper.cpp/source/models/for-tests-ggml-tiny.bin\"));",
            "const T: &str = include_str!(\"..\\\\vendor\\\\whisper.cpp\\\\source\\\\models\\\\x.txt\");",
            // rustfmt's form for a long embed: the path on a line of its own.
            "static D: &[u8] = include_bytes!(concat!(\n    env!(\"CARGO_MANIFEST_DIR\"),\n    \"/../../vendor/whisper.cpp/source/models/for-tests-silero-v6.2.0-ggml.bin\"\n));",
            // A long literal moved to the line after the `=`.
            "static E: &[u8] =\n    include_bytes!(\"../../vendor/whisper.cpp/source/models/ggml-silero.bin\");",
            // A long literal moved to the line after the parenthesis.
            "static F: &[u8] = include_bytes!(\n    \"../../models/ggml-silero-v6.2.0.bin\"\n);",
            // A test module declared early does not hide the code after it.
            "#[cfg(test)]\nmod tests;\nconst M: &[u8] = include_bytes!(\"m.bin\");",
            // A test item that spans lines ends where its braces close.
            "#[cfg(test)]\nfn f() {\n    let _ = '}';\n}\nconst M: &[u8] = include_bytes!(\"m.bin\");",
        ];
        for source in planted {
            assert_eq!(embedded_models(source).len(), 1, "{source}");
        }
        let allowed = "const J: &[u8] = include_bytes!(\"../../../vendor/whisper.cpp/source/samples/jfk.wav\");\n\
                       #[cfg(test)]\nconst F: &[u8] = include_bytes!(\"m.bin\");\n\
                       #[cfg(test)]\n#[allow(dead_code)]\nconst G: &[u8] =\n    include_bytes!(\"n.bin\");\n\
                       // const H: &[u8] = include_bytes!(\"commented-out.bin\");\n\
                       const S: &str = \"include_bytes!(\\\"in-a-string.bin\\\")\";\n\
                       #[cfg(test)]\nmod tests {\n    const K: &[u8] = include_bytes!(\"k.bin\");\n    fn g() -> char { '\\\\' }\n}";
        assert_eq!(embedded_models(allowed), Vec::<String>::new());
    }

    #[cfg(target_os = "macos")]
    fn tool_output(tool: &str, args: &[&str], exe: &Path) -> String {
        let output = std::process::Command::new(tool)
            .args(args)
            .arg(exe)
            .output()
            .unwrap_or_else(|e| panic!("{tool} could not run: {e}"));
        assert!(output.status.success(), "{tool} failed");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}
