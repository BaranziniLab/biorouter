//! The guard behind the per-binary test sandbox.
//!
//! `src/test_sandbox.rs` redirects this crate's config/data dirs at a throwaway
//! root before `main`, so a test can never open the developer's real
//! `config.yaml` or `sessions.db`. The lib declares it `#[cfg(test)]`, which
//! means it is **not** compiled into integration test binaries — each
//! `tests/*.rs` is its own crate and must declare its own copy.
//!
//! That makes the protection opt-in, and opt-in protection is only as good as
//! the thing that notices when someone forgets. In `biorouter-server` nothing
//! did: one branch enumerated the integration files that existed when it was
//! written, another added a tenth afterwards, and the merge produced a binary
//! with no sandbox. Neither branch was wrong on its own — the seam between them
//! was. So the rule is enforced here rather than remembered, before this crate
//! has the chance to repeat it.
//!
//! ⚠ The name is `cli_test_binaries_are_sandboxed`, not the `biorouter-server`
//! sibling's `every_test_binary_is_sandboxed`, because `rust.yml`'s "integration
//! binaries" step selects targets with `--test <name>`, which matches across the
//! whole workspace. It refuses to run at all when two packages share an
//! integration target name (`::error::two packages have an integration target
//! named: …`), so the obvious name is a red CI run, not a duplicate test.

// This file opens no database, but it takes the rule it enforces — an exception
// here would be the first crack in it.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

/// Every top-level file in `tests/` is a separate binary, so every one needs its
/// own `mod test_sandbox;`. Subdirectories are shared helper modules, not
/// binaries, and are correctly skipped.
#[test]
fn every_integration_test_binary_declares_the_sandbox() {
    let tests_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");

    let mut checked = Vec::new();
    let mut missing = Vec::new();

    for entry in std::fs::read_dir(&tests_dir).expect("crates/biorouter-cli/tests is readable") {
        let path = entry.expect("a readable directory entry").path();
        // Only top-level `.rs` files become test binaries.
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let name = path
            .file_name()
            .expect("a file has a name")
            .to_string_lossy()
            .into_owned();
        let source = std::fs::read_to_string(&path).expect("a test file is readable");

        checked.push(name.clone());
        if !source.contains("mod test_sandbox;") {
            missing.push(name);
        }
    }

    assert!(
        !checked.is_empty(),
        "found no test binaries to check: this guard has stopped guarding anything, \
         which is worse than the defect it exists to catch"
    );

    missing.sort();
    assert!(
        missing.is_empty(),
        "these integration test binaries do not declare the test sandbox, so anything \
         in them that reaches Config::global() or SessionManager::instance() resolves \
         the DEVELOPER'S REAL config and sessions.db: {missing:?}\n\nAdd these two \
         lines below the file's inner attributes:\n\n    \
         #[path = \"../src/test_sandbox.rs\"]\n    mod test_sandbox;\n\n\
         (The lib's copy is #[cfg(test)] and is not compiled into integration binaries.)"
    );
}
