//! Biorouter's local dictation engine (`docs/dictation/engine-and-models.md`).
//!
//! whisper.cpp, vendored under `vendor/whisper.cpp`, is compiled into every shipped
//! `biorouter` and `biorouterd` (DD-2, DD-5) and driven through the C++ shim in `shim/`.
//! It only ever runs in a worker process ([`worker`]), never inside the daemon or an
//! interactive CLI (DD-4). The CPU gate ([`cpu`]) runs before any worker starts.
//!
//! A leaf crate: no Biorouter dependency, and every path is an argument.

pub mod build_info;
pub mod cpu;
#[cfg(all(feature = "engine", not(br_no_native)))]
pub mod engine;
/// Exists whenever the feature is on, so both `main()` functions can call
/// `worker::run_if_invoked()`; under `br_no_native` it answers 2 for every role.
#[cfg(feature = "engine")]
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
