//! Task 30, matrix row 15 (`archive`): the forced export location follows the
//! master privacy toggle (issue #56, DR-15).
//!
//! ⚠ **Why this is an integration binary and not a `#[cfg(test)] mod` beside
//! `kb_export`.** The toggle is a PROCESS-GLOBAL atomic and `cargo test` runs a
//! crate's unit tests in parallel threads of one process. Written next to the
//! code it exercises, this test's OFF window disabled the barrier under
//! `no_tool_that_names_a_base_reaches_a_private_one_under_a_public_model` and
//! `an_imported_base_takes_the_importing_sessions_tier_or_the_archives_floor`,
//! which do not take the guard and cannot be made to — there are ~250 of them.
//! Measured, not predicted: both failed on the first run. Each `tests/*.rs` file
//! is its own process, so nothing outside this file can observe its writes.
//!
//! ⚠ **Why the fixture is rebuilt rather than reused.** `KnowledgeServer`'s
//! fields are private and `server_with_root` is a `#[cfg(test)]` helper of the
//! module, so an integration test cannot root a server at a temp directory
//! directly. It goes through `KnowledgeServer::new()`, which resolves its root
//! from `BIOROUTER_PATH_ROOT` — the same resolution the shipped daemon uses.

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use biorouter_mcp::knowledge::server::{ExportArchiveParams, KnowledgeServer};
use rmcp::handler::server::wrapper::Parameters;

/// Restores the toggle on drop, including on the unwind path a failing
/// assertion takes.
struct Restore(bool);

impl Drop for Restore {
    fn drop(&mut self) {
        biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(self.0);
    }
}

fn reported_path(out: &rmcp::model::CallToolResult) -> std::path::PathBuf {
    let text = out
        .content
        .iter()
        .find_map(|c| c.as_text())
        .expect("kb_export returns a text payload");
    let v: serde_json::Value = serde_json::from_str(&text.text).expect("valid json");
    std::path::PathBuf::from(v["path"].as_str().expect("a reported path"))
}

async fn export(srv: &KnowledgeServer, kb_id: &str, dest: &std::path::Path) -> std::path::PathBuf {
    reported_path(
        &srv.kb_export(Parameters(ExportArchiveParams {
            kb_id: kb_id.to_string(),
            dest_path: Some(dest.display().to_string()),
        }))
        .await
        .expect("a private caller may export"),
    )
}

#[tokio::test]
async fn the_forced_export_location_follows_the_master_toggle() {
    let _restore = Restore(biorouter_mcp::privacy_toggle::privacy_tiers_enabled());
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(true);

    let tmp = tempfile::tempdir().unwrap();
    let _env = env_lock::lock_env([(
        "BIOROUTER_PATH_ROOT",
        Some(tmp.path().to_str().expect("utf-8 temp path")),
    )]);
    let srv = KnowledgeServer::new().expect("a server rooted under the temp path");
    let root = biorouter_mcp::knowledge::paths::knowledge_root().unwrap();

    // A real private base with a page in it, created through the production path
    // that stamps its tier in the same transaction. A second `KnowledgeService`
    // over the SAME root, because `KnowledgeServer::service` is private — the
    // store is on disk, so the server built above sees it.
    biorouter_mcp::knowledge::service::KnowledgeService::new(root.clone())
        .create_base_as(
            "omop",
            "OMOP",
            None,
            biorouter_mcp::knowledge::types::KbFormat::default(),
            /* caller_is_private */ true,
            // DR-26's axis is not what this test is about, and `Unstated` adds
            // no owner — so the forced-export-location assertions below are
            // still about the TIER and nothing else.
            &biorouter_mcp::knowledge::affiliation::CallerAffiliation::Unstated,
        )
        .expect("create the private base");
    assert!(biorouter_mcp::knowledge::tier::is_private(&root, "omop"));
    let page = root.join("omop").join("knowledge").join("x.md");
    std::fs::create_dir_all(page.parent().unwrap()).unwrap();
    std::fs::write(&page, "SENTINEL-COHORT-N-412").unwrap();

    let elsewhere = tempfile::tempdir().unwrap();
    let asked = elsewhere.path().join("omop.brkb");

    // ON: the model does not choose where a private base's archive lands.
    let written = export(&srv, "omop", &asked).await;
    assert!(
        written.starts_with(biorouter_mcp::knowledge::paths::model_export_dir(&root)),
        "reported {}, which is not inside the knowledge root",
        written.display()
    );
    assert!(
        !asked.exists(),
        "the archive was written where the model aimed it"
    );

    // OFF: nothing is relocated. The archive goes where it was asked to go, and
    // the tier stamp already written is NOT rewritten — turning the feature off
    // stops enforcement, it does not erase a classification (AR-7).
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(false);
    assert!(biorouter_mcp::knowledge::tier::is_private(&root, "omop"));
    let written = export(&srv, "omop", &asked).await;
    assert_eq!(written, asked);
    assert!(asked.exists());
}
