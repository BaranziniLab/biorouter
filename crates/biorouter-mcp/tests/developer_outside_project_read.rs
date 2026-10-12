//! End-to-end confinement: a pooled developer server never inherits another
//! session's permission mode, so `text_editor` remains inside its bound project.

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use biorouter_mcp::developer::rmcp_developer::TextEditorParams;
use biorouter_mcp::DeveloperServer;
use rmcp::handler::server::wrapper::Parameters;

fn view_params(path: &str) -> Parameters<TextEditorParams> {
    Parameters(TextEditorParams {
        path: path.to_string(),
        command: "view".to_string(),
        diff: None,
        view_range: None,
        file_text: None,
        old_str: None,
        new_str: None,
        insert_line: None,
    })
}

#[tokio::test(flavor = "current_thread")]
async fn text_editor_refuses_reads_outside_the_bound_project() {
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();

    // The project protects its own `notes.txt`...
    std::fs::write(project.path().join(".biorouterignore"), "notes.txt\n").unwrap();
    let project_secret = project.path().join("notes.txt");
    std::fs::write(&project_secret, "internal").unwrap();

    // ...while an unrelated directory happens to hold a file of the same name.
    let stranger = outside.path().join("notes.txt");
    std::fs::write(&stranger, "READ ALLOWED").unwrap();

    let server = DeveloperServer::new().with_working_dir(project.path().to_path_buf());

    // Sanity: the project's own rule is still enforced, so this test cannot pass
    // by the guard simply doing nothing.
    let refused = server
        .text_editor(view_params(project_secret.to_str().unwrap()))
        .await
        .expect_err("the project's own .biorouterignore must still deny its file");
    assert!(
        refused.message.contains(".biorouterignore"),
        "the refusal must name the reason, got: {}",
        refused.message
    );

    let refused = server
        .text_editor(view_params(stranger.to_str().unwrap()))
        .await
        .expect_err("an outside file must remain outside the text-editor jail");
    assert!(
        refused.message.contains("outside the working directory"),
        "the refusal must name the containment boundary: {}",
        refused.message
    );
}
