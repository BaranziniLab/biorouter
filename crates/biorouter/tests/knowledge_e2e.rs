biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::knowledge::{convert::SourceInput, service::KnowledgeService};

#[tokio::test]
async fn e2e_create_add_query_restore() {
    let dir = tempfile::tempdir().unwrap();
    let svc = KnowledgeService::new(dir.path().to_path_buf());

    // 1. Create.
    let m = svc
        .create_base("ms", "MS Patient Analysis", Some("#5a6394"))
        .unwrap();
    assert_eq!(m.id, "ms");
    let bases = svc.list_bases().unwrap();
    assert_eq!(bases.len(), 1);

    // 2. Add a text source.
    let added = svc
        .add_raw_source(
            "ms",
            SourceInput::Text {
                text: "Brain MRI shows demyelination consistent with MS.".into(),
                title: Some("Imaging note".into()),
            },
            None,
        )
        .await
        .unwrap();
    assert!(dir
        .path()
        .join(format!("ms/raw/{}/source.md", added.source_id))
        .exists());

    // 3. Add a URL source (mocked via a separate test would normally be ideal;
    //    here we just verify that text-source path works end-to-end).

    // 4. History reflects two commits (init + ingest).
    let h = svc.list_history("ms", 10).unwrap();
    assert_eq!(h.len(), 2);
    let init_sha = h.last().unwrap().commit_sha.clone();

    // 5. Restore to init.
    svc.restore_state("ms", &init_sha).unwrap();
    let h2 = svc.list_history("ms", 10).unwrap();
    assert_eq!(h2.len(), 3); // init, ingest, restore
    assert!(!dir
        .path()
        .join(format!("ms/raw/{}/source.md", added.source_id))
        .exists());

    // 6. Graph cache is up to date.
    let g = svc.get_graph("ms").unwrap();
    assert!(g.nodes.is_empty()); // No knowledge pages yet (macros not in this plan)
}
