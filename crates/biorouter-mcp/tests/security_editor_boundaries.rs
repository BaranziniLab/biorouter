//! Audit replay through the public editor tool using synthetic workspaces.
#![cfg(unix)]

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use biorouter_mcp::developer::rmcp_developer::TextEditorParams;
use biorouter_mcp::DeveloperServer;
use rmcp::handler::server::wrapper::Parameters;
use serde_json::json;

fn edit(path: &str, diff: bool) -> Parameters<TextEditorParams> {
    Parameters(TextEditorParams {
        path: if diff { "." } else { path }.into(),
        command: if diff { "str_replace" } else { "write" }.into(),
        diff: diff.then(|| {
            format!("--- /dev/null\n+++ b/{path}\n@@ -0,0 +1 @@\n+SYNTHETIC_AUDIT_CONTENT\n")
        }),
        view_range: None,
        file_text: (!diff).then(|| "SYNTHETIC_AUDIT_CONTENT".into()),
        old_str: None,
        new_str: None,
        insert_line: None,
    })
}

#[tokio::test]
async fn public_editor_replay_preserves_workspace_boundaries_and_normal_edits() {
    use std::os::unix::fs::symlink;

    for diff in [false, true] {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("workspace");
        let outside = fixture.path().join("outside");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&outside).unwrap();
        symlink(outside.join("leaf.txt"), workspace.join("file-link")).unwrap();
        symlink(outside.join("missing-dir"), workspace.join("dir-link")).unwrap();
        symlink(&outside, workspace.join("outside-link")).unwrap();
        let server = DeveloperServer::new().with_working_dir(workspace.clone());
        let mut unsafe_paths = vec![
            "file-link",
            "dir-link/new.txt",
            "outside-link/missing/new.txt",
        ];
        if diff {
            unsafe_paths.push("workspace/outside-link/missing/new.txt");
        }
        for path in unsafe_paths {
            let result = server.text_editor(edit(path, diff)).await;
            assert!(result.is_err(), "unsafe path accepted: {path}");
            assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
            println!(
                "EDITOR_AUDIT {}",
                json!({"operation":if diff {"diff_creation"} else {"direct_write"},"path":path,"denied":true,"outside_entries":0})
            );
        }
        let valid = if diff {
            "normal/nested/new.txt"
        } else {
            "normal.txt"
        };
        server.text_editor(edit(valid, diff)).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(workspace.join(valid)).unwrap(),
            "SYNTHETIC_AUDIT_CONTENT\n"
        );
        println!(
            "EDITOR_AUDIT {}",
            json!({"operation":if diff {"diff_creation"} else {"direct_write"},"control":"normal_file","allowed":true})
        );
        if diff {
            server
                .text_editor(edit("workspace/overlap/nested/new.txt", true))
                .await
                .unwrap();
            assert_eq!(
                std::fs::read_to_string(workspace.join("overlap/nested/new.txt")).unwrap(),
                "SYNTHETIC_AUDIT_CONTENT\n"
            );
            println!(
                "EDITOR_AUDIT {}",
                json!({"operation":"diff_creation","control":"overlap_prefixed_internal_nested_file","allowed":true})
            );
        }
        if !diff {
            symlink(workspace.join(valid), workspace.join("internal-link")).unwrap();
            server
                .text_editor(edit("internal-link", false))
                .await
                .unwrap();
            println!(
                "EDITOR_AUDIT {}",
                json!({"operation":"direct_write","control":"valid_internal_symlink","allowed":true})
            );
        }
    }
}
