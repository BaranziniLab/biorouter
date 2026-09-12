//! Two DIFFERENT session stores in one process never mint the same session id —
//! in a build of `biorouter` without `cfg(test)`.
//!
//! `session_manager.rs`'s own `two_stores_in_one_process_never_mint_the_same_id`
//! asserts this property too, and could not see the defect this file was written
//! for: a lib unit test is compiled WITH `cfg(test)`, and until 2026-09-12 that
//! was the only configuration in which `SessionStorage::id_prefix` gave each
//! store its own prefix. Every integration binary in the workspace — this one,
//! the rest of `crates/biorouter/tests/`, and every test in `biorouter-server`,
//! `biorouter-mcp` and `biorouter-cli`, all of which CI runs — links the crate
//! built without it, prefixed ids with the date, and minted `<date>_1` from
//! every store.
//!
//! Why that is worth a binary of its own rather than a line in an existing one:
//! a session id is the key of several PROCESS-GLOBAL registries
//! (`agents::subagent_handle::HANDLES`, `session_events`' bus, `AgentManager`'s
//! pin), so the second store's chat silently inherits the first's entries and a
//! turn waits forever on work that has already finished. The symptom is a hang
//! in a test that has nothing to do with session ids, tens of minutes later, with
//! nothing in the log naming a cause — the shape that cost this repository whole
//! CI jobs (commit acae89ea, and ~4000 discarded results on #273).
//!
//! Measured on this file, 2026-09-12:
//!   * before the fix: `the_second_store_in_this_process_mints_ids_of_its_own`
//!     fails with `both stores minted "20260912_1"`.
//!   * after: the second store mints `s0000001_1` and the first still mints
//!     `20260912_1`.
//!
//! ⚠ **One `#[test]`, deliberately.** The date prefix belongs to the FIRST store
//! this process mints from, and "first" is decided by whichever thread the
//! harness schedules first. Split across several test functions, the assertion
//! that production's id shape is intact would pass or fail by luck. One function
//! owns the order.

use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use tempfile::TempDir;

/// The `N` of a `PREFIX_N` id.
fn suffix(id: &str) -> i64 {
    id.rsplit('_')
        .next()
        .unwrap_or_else(|| panic!("`{id}` is not a PREFIX_N session id"))
        .parse()
        .unwrap_or_else(|e| panic!("`{id}` has an unparseable counter: {e}"))
}

/// The `PREFIX` of a `PREFIX_N` id.
fn prefix(id: &str) -> &str {
    let (head, _) = id
        .rsplit_once('_')
        .unwrap_or_else(|| panic!("`{id}` is not a PREFIX_N session id"));
    head
}

async fn mint(sm: &SessionManager, dir: &TempDir, n: usize) -> Vec<String> {
    let mut ids = Vec::with_capacity(n);
    for _ in 0..n {
        ids.push(
            sm.create_session(dir.path().to_path_buf(), "c".into(), SessionType::User)
                .await
                .unwrap()
                .id,
        );
    }
    ids
}

#[tokio::test]
async fn the_second_store_in_this_process_mints_ids_of_its_own() {
    // ── The first store: production's only one, and its ids must not move. ──
    let first_dir = TempDir::new().unwrap();
    let first = SessionManager::new(first_dir.path().to_path_buf());
    let first_ids = mint(&first, &first_dir, 3).await;

    let today = chrono::Utc::now().format("%Y%m%d").to_string();
    assert_eq!(
        first_ids.iter().map(|id| prefix(id)).collect::<Vec<_>>(),
        vec![today.as_str(); 3],
        "the process's first store must still mint YYYYMMDD_N — that is every \
         shipped id, and `{today}` is today. Got {first_ids:?}"
    );
    assert_eq!(
        first_ids.iter().map(|id| suffix(id)).collect::<Vec<_>>(),
        vec![1, 2, 3],
        "one store must still number 1..n with no gaps; got {first_ids:?}"
    );

    // ── A second, DIFFERENT store. This is the whole defect. ──
    let second_dir = TempDir::new().unwrap();
    let second = SessionManager::new(second_dir.path().to_path_buf());
    let second_ids = mint(&second, &second_dir, 3).await;

    for id in &second_ids {
        assert!(
            !first_ids.contains(id),
            "both stores minted `{id}` (first store {first_ids:?}, second \
             {second_ids:?}). A session id keys process-global registries — \
             subagent_handle::HANDLES, session_events, AgentManager's pin — so \
             the second store's chat inherits the first's entries and a turn \
             that waits on one of them never returns.",
        );
    }
    assert_ne!(
        prefix(&second_ids[0]),
        today,
        "the second store took the date prefix, so it restarts the same counter \
         over its own empty database and collides id for id: {second_ids:?}"
    );

    // ── A third one, so the rule is a rule and not a two-store special case. ──
    let third_dir = TempDir::new().unwrap();
    let third = SessionManager::new(third_dir.path().to_path_buf());
    let third_ids = mint(&third, &third_dir, 3).await;

    let minted: Vec<&String> = first_ids
        .iter()
        .chain(second_ids.iter())
        .chain(third_ids.iter())
        .collect();
    let distinct: std::collections::HashSet<&&String> = minted.iter().collect();
    assert_eq!(
        distinct.len(),
        minted.len(),
        "three stores in one process minted a duplicate id: {minted:?}"
    );

    // ── Re-opening a store keeps its prefix, so its counter cannot restart. ──
    //
    // A prefix bound to the manager rather than to the directory would hand this
    // reopened store a fresh one, and it would then mint `<new>_1` over a
    // database whose ids are all `<old>_N` — a collision with its own future.
    let reopened = SessionManager::new(second_dir.path().to_path_buf());
    let reopened_ids = mint(&reopened, &second_dir, 2).await;
    assert_eq!(
        reopened_ids.iter().map(|id| prefix(id)).collect::<Vec<_>>(),
        vec![prefix(&second_ids[0]); 2],
        "re-opening a store changed its prefix: it minted {reopened_ids:?} over \
         a database already holding {second_ids:?}"
    );
    assert_eq!(
        reopened_ids.iter().map(|id| suffix(id)).collect::<Vec<_>>(),
        vec![4, 5],
        "a re-opened store must carry on counting, not restart; got {reopened_ids:?}"
    );
}
