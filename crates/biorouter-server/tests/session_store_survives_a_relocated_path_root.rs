//! A test that relocates `BIOROUTER_PATH_ROOT` must not be able to move this
//! binary's `sessions.db` into a directory it then deletes.
//!
//! This is the regression test for a `test (windows-latest)` flake that rotated
//! through the `routes::session` tests — `activity_clamps_an_absurd_window`
//! answering `500` where it wanted `200`, `sidebar_route_…` and the `/usage`
//! routes on other runs. Every one of them is a read-only route test, and the
//! 500 is `get_activity(..).map_err(|_| INTERNAL_SERVER_ERROR)`: a genuine
//! database error, reported correctly, from a database that had been moved out
//! from under the process.
//!
//! The sequence this file reproduces:
//!
//! 1. `BIOROUTER_PATH_ROOT` is process-global, and a test may relocate it under
//!    a `TempDir` to get its own config/skills/knowledge root — `routes::apps`'s
//!    `lock_env_for` and `routes::config_management`'s privacy tests do exactly
//!    that, for good reasons.
//! 2. The process-global session store resolves its directory the first time
//!    anything touches it. The tests run in parallel, so that first touch could
//!    be made by a test holding such a lock — and then the whole binary's
//!    `sessions.db` lives inside that test's `TempDir`.
//! 3. The `TempDir` drops and the directory is unlinked.
//! 4. **Nothing breaks yet.** The connection the pool already opened keeps
//!    answering: SQLite on POSIX does not care that its inode lost its name. A
//!    serial query still returns 200, which is why this hid for so long.
//! 5. The first time two tasks want the pool at once it must open a *second*
//!    connection — and that fails with `(code: 14) unable to open database
//!    file`. Whichever test was querying then fails, so the name rotates, and
//!    the same test can pass in the `biorouter_server` lib binary and fail in
//!    the `biorouterd` bin binary a minute later.
//!
//! The fix is in `src/test_sandbox.rs`: the `#[ctor]` freezes the store's
//! directory at the sandbox root before any test runs, so step 2 can no longer
//! pick a doomed one. This file asserts steps 1–5 are now harmless, and it fails
//! (15 of 16 concurrent queries erroring) if that pin is removed.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use biorouter::session::session_manager::SessionManager;

/// Stand in for a sibling test that relocates the path root: pin the store from
/// inside the lock, unlink the directory, then use the store the way the routes
/// do — including concurrently, which is what forces a second connection.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_relocated_path_root_cannot_move_the_session_store() {
    // Read the sandbox from the ENVIRONMENT, not from `shared_store_root()`:
    // calling that here would freeze the store at the right answer even with the
    // ctor's pin removed, and the test would pass while proving nothing.
    let sandbox = std::path::PathBuf::from(
        std::env::var("BIOROUTER_PATH_ROOT").expect("the ctor sandboxes this binary"),
    )
    .join("data");
    let temp = tempfile::TempDir::new().unwrap();
    let temp_root = temp.path().to_path_buf();

    {
        let _env = env_lock::lock_env([(
            "BIOROUTER_PATH_ROOT",
            Some(temp_root.to_str().expect("utf-8 temp path")),
        )]);

        // The relocation is real — an unpinned resolver would follow it.
        assert!(
            biorouter::config::paths::Paths::data_dir().starts_with(&temp_root),
            "the env lock did not take, so this test proves nothing"
        );

        // …but the store does not, and it is the store a query follows.
        assert_eq!(
            SessionManager::shared_store_root(),
            sandbox.as_path(),
            "a test that relocates BIOROUTER_PATH_ROOT moved the process session store"
        );

        SessionManager::instance()
            .get_activity(30)
            .await
            .expect("a query made while the path root is relocated");
    }

    drop(temp);
    assert!(
        !temp_root.exists(),
        "the temp root outlived its TempDir, so the unlink half is untested"
    );

    // Serial first: this half passed even before the fix, and saying so here is
    // what stops a future reader from deleting the concurrent half as redundant.
    SessionManager::instance()
        .get_activity(30)
        .await
        .expect("a serial query after the relocated root was unlinked");

    // Concurrent: the pool has `max_connections(4)`, so this is the half that
    // actually needs to OPEN a connection against the pinned directory. Pinned
    // to an unlinked TempDir, 15 of these 16 fail with SQLITE_CANTOPEN.
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..16 {
        tasks.spawn(async { SessionManager::instance().get_activity(30).await });
    }
    let mut failures = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        if let Err(e) = joined.expect("task panicked") {
            failures.push(e.to_string());
        }
    }
    assert!(
        failures.is_empty(),
        "{} of 16 concurrent queries failed after a sibling's path root was \
         unlinked — the session store is pinned outside the sandbox. First: {}",
        failures.len(),
        failures.first().map(String::as_str).unwrap_or("")
    );
}
