//! End-to-end integration test for `KnowledgeService::restore_state`.
//!
//! Verifies the lower-level revert behaviour: after creating a KB, writing
//! page A, committing, writing page B, committing, and then calling
//! `restore_state` back to the SHA captured after page A, the working tree
//! contains page A but not page B and `list_history` shows the newest entry
//! as a `Restore`.
//!
//! This is a direct service-level integration test — no HTTP, no MCP — and
//! acts as a regression guard for the file-presence + history-kind contract
//! that higher-level routes rely on.

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use biorouter_mcp::knowledge::{git::GitRepo, paths, service::KnowledgeService, types::ChangeKind};

#[test]
fn restore_state_reverts_a_page_creation() {
    // ---- setup ------------------------------------------------------------
    let dir = tempfile::tempdir().unwrap();
    let svc = KnowledgeService::new(dir.path().to_path_buf());
    let kb_id = "rev";
    svc.create_base(kb_id, "Revert Test", None).unwrap();

    let kb_root = paths::kb_root(svc.root(), kb_id);
    let notes_dir = paths::kb_knowledge_dir(svc.root(), kb_id).join("note");
    std::fs::create_dir_all(&notes_dir).unwrap();
    let page_a = notes_dir.join("page_a.md");
    let page_b = notes_dir.join("page_b.md");

    // ---- write & commit page A -------------------------------------------
    std::fs::write(&page_a, "# Page A\n\nfirst note\n").unwrap();
    let repo = GitRepo::open(&kb_root).unwrap();
    repo.commit_all(ChangeKind::Manual, "add page_a", None)
        .unwrap();

    let history_after_a = svc.list_history(kb_id, 10).unwrap();
    // create-base + add-page-a = 2 entries; newest first.
    assert_eq!(history_after_a.len(), 2, "expected 2 commits after page A");
    let sha_after_a = history_after_a[0].commit_sha.clone();

    // ---- write & commit page B -------------------------------------------
    std::fs::write(&page_b, "# Page B\n\nsecond note\n").unwrap();
    repo.commit_all(ChangeKind::Manual, "add page_b", None)
        .unwrap();
    assert!(page_a.exists() && page_b.exists(), "both pages on disk");

    // ---- restore back to the post-A commit -------------------------------
    let restore_sha = svc.restore_state(kb_id, &sha_after_a).unwrap();
    assert!(!restore_sha.is_empty(), "restore returned an empty sha");

    // ---- file-presence assertions ----------------------------------------
    assert!(
        page_a.exists(),
        "page A should still be present after restore"
    );
    assert!(
        !page_b.exists(),
        "page B should be gone after restoring to a commit that predates it"
    );

    // ---- history assertions ----------------------------------------------
    let history_after_restore = svc.list_history(kb_id, 10).unwrap();
    // create + page_a + page_b + restore = 4 entries.
    assert_eq!(
        history_after_restore.len(),
        4,
        "history should append a restore commit, not rewrite"
    );
    assert_eq!(
        history_after_restore[0].kind,
        ChangeKind::Restore,
        "newest history entry must be a Restore"
    );
}
