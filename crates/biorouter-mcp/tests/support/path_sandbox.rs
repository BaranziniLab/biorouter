//! Included by every integration binary in this directory (`#[path]`), and
//! required of each by `every_integration_binary_is_sandboxed` below: points
//! `BIOROUTER_PATH_ROOT` at a throwaway directory before `main`, unless a root
//! is already in force.
//!
//! Without it a test that reaches a resolver reads the developer's real
//! folders, and on Windows the resolver also holds the lock beside the folder
//! earlier releases used (R-31, `biorouter_mcp::app_folder`). Binaries that
//! set a root of their own under `env_lock` still do; this is only the default
//! underneath them. Same predicate as every resolver
//! (`biorouter_mcp::paths::path_root_override`): unset, blank and not UTF-8
//! count as no root.

/// Never dropped: any resolution in the binary may land here.
static SANDBOX_ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();

#[ctor::ctor]
fn sandbox_this_integration_binary() {
    if biorouter_mcp::paths::path_root_override().is_some() {
        return;
    }
    match tempfile::TempDir::new() {
        Ok(root) => {
            std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
            let _ = SANDBOX_ROOT.set(root);
        }
        Err(error) => {
            // A panic cannot unwind out of a ctor, and the message is the point.
            eprintln!(
                "test sandbox: could not create a temporary directory ({error}); without one \
                 a test would read the developer's real config and data directories. Fix the \
                 temp directory, or export BIOROUTER_PATH_ROOT."
            );
            std::process::abort();
        }
    }
}

/// Every `tests/*.rs` includes this file, so a new binary cannot run
/// unsandboxed. Asserted from each binary that includes it, which is all of
/// them; one failing is enough.
#[test]
fn every_integration_binary_is_sandboxed() {
    assert!(
        biorouter_mcp::paths::path_root_override().is_some(),
        "this binary runs without a BIOROUTER_PATH_ROOT"
    );
    let tests = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let include = ["#[path = \"support/path_", "sandbox.rs\"]"].concat();
    let mut missing = Vec::new();
    for entry in std::fs::read_dir(&tests).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "rs")
            && !std::fs::read_to_string(&path).unwrap().contains(&include)
        {
            missing.push(path.display().to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "these integration binaries do not include tests/support/path_sandbox.rs: {missing:?}"
    );
}
