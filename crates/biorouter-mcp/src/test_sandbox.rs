//! Sandbox this crate's lib test binary before any test runs (R-31).
//!
//! The `biorouter`, `biorouter-cli` and `biorouter-server` lib test binaries
//! each point `BIOROUTER_PATH_ROOT` at a throwaway directory in a `#[ctor]`.
//! This one did not, so a test that reached a resolver (for instance
//! `paths::tests::in_config_dir_appends_the_subdir`, which exercises the real
//! one) read the developer's real folders, and on Windows the resolver also
//! takes the lock beside the folder earlier releases used
//! ([`crate::app_folder`]). Under a root of its own, `platform_dirs` looks at
//! neither, and no store resolves outside the sandbox.
//!
//! An outer root wins, read with the predicate every resolver uses
//! ([`crate::paths::path_root_override`]): unset, blank and not UTF-8 all count
//! as no root, so a blank value cannot pass for a sandbox here while the
//! resolvers read the real folders.

use std::sync::OnceLock;

/// Never dropped: every resolution in the binary may land here.
static SANDBOX_ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();

#[ctor::ctor]
fn sandbox_this_crates_lib_test_binary() {
    if crate::paths::path_root_override().is_some() {
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
                "biorouter-mcp test sandbox: could not create a temporary directory ({error}); \
                 without one every test would read the developer's real config and data \
                 directories. Fix the temp directory, or export BIOROUTER_PATH_ROOT."
            );
            std::process::abort();
        }
    }
}

#[test]
fn this_binary_never_reaches_the_real_app_folder() {
    assert!(
        crate::paths::path_root_override().is_some(),
        "the lib test binary runs without a BIOROUTER_PATH_ROOT, so every resolver reaches the \
         developer's real folders"
    );
    // Resolve the way every store does.
    let _ = crate::paths::platform_dirs();
    let _ = crate::paths::config_dir();
    let _ = crate::knowledge::paths::knowledge_root();
    assert!(
        crate::app_folder::process_move().is_none(),
        "a resolver ran the Windows folder move in a test binary; only `biorouterd agent` may"
    );
    assert!(
        !crate::app_folder::process_holds_a_lock(),
        "a test took the lock beside the real folder earlier releases used"
    );
}
