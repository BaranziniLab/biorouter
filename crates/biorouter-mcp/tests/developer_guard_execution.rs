//! Real MCP dispatch probes using private synthetic files, never account credentials.
#![cfg(unix)]

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use biorouter_mcp::DeveloperServer;
use rmcp::model::{CallToolRequestParams, CallToolResult, RawContent};
use rmcp::ServiceExt;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::timeout;

const CANARY: &str = "SYNTHETIC_GUARD_CANARY_not_an_account_key";
type Outcome = Result<CallToolResult, String>;

async fn call_real_tools(root: &Path, calls: Vec<(&str, Value)>) -> Vec<Outcome> {
    let server = DeveloperServer::new()
        .with_working_dir(root.to_path_buf())
        .with_foreground_timeout(Some(Duration::from_secs(2)));
    let (client_io, server_io) = tokio::io::duplex(65536);
    let mut serving =
        tokio::spawn(async move { server.serve(server_io).await.unwrap().waiting().await });
    let connected = timeout(Duration::from_secs(5), ().serve(client_io)).await;
    let client = match connected {
        Ok(Ok(client)) => client,
        _ => {
            serving.abort();
            let _ = serving.await;
            panic!("private MCP handshake did not complete");
        }
    };
    let mut outcomes = Vec::new();
    for (name, arguments) in calls {
        let result = timeout(
            Duration::from_secs(5),
            client.call_tool(CallToolRequestParams {
                meta: None,
                name: name.to_owned().into(),
                arguments: Some(arguments.as_object().unwrap().clone()),
                task: None,
            }),
        )
        .await;
        outcomes.push(match result {
            Ok(result) => result.map_err(|error| error.to_string()),
            Err(_) => Err("MCP_CALL_DEADLINE_EXCEEDED".to_owned()),
        });
    }
    let closed = timeout(Duration::from_secs(3), client.cancel()).await;
    let joined = timeout(Duration::from_secs(3), &mut serving).await;
    if joined.is_err() {
        serving.abort();
        let _ = serving.await;
    }
    assert!(
        matches!(closed, Ok(Ok(_))),
        "MCP client must close normally"
    );
    assert!(
        matches!(joined, Ok(Ok(Ok(_)))),
        "MCP server must end normally before checking the behavioral oracles"
    );
    outcomes
}

fn shell(command: &str) -> (&str, Value) {
    ("shell", json!({"command": command, "background": false}))
}

fn editor(path: &str) -> (&str, Value) {
    ("text_editor", json!({"command": "view", "path": path}))
}

fn text(outcome: &Outcome) -> String {
    let result = outcome.as_ref().expect("the real tool call must succeed");
    assert_ne!(result.is_error, Some(true));
    result
        .content
        .iter()
        .filter_map(|content| match &content.raw {
            RawContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn refused(outcome: &Outcome) {
    let error = outcome
        .as_ref()
        .expect_err("the guard must refuse the real tool call");
    assert!(
        error.contains("blocked") || error.contains("secret-file policy"),
        "a transport, schema, timeout, or ordinary command error is not a guard refusal: {error}"
    );
    assert!(!error.contains(CANARY));
}

fn fixture() -> TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("config")).unwrap();
    std::fs::write(root.path().join("config/.env"), CANARY).unwrap();
    std::fs::write(root.path().join("config/.env.example"), "REGION=fixture\n").unwrap();
    root
}

#[tokio::test]
async fn issue522_real_shell_allows_a_regular_template_but_refuses_credential_copy() {
    let root = fixture();
    let outcomes = call_real_tools(
        root.path(),
        vec![
            shell("cp config/.env.example copied-template.txt"),
            editor("config/.env.example"),
            editor("config/.env"),
            shell("printf launched > secret-launch-marker; cat config/.env > stolen.txt"),
        ],
    )
    .await;
    text(&outcomes[0]);
    assert_eq!(
        std::fs::read(root.path().join("copied-template.txt")).unwrap(),
        b"REGION=fixture\n"
    );
    assert!(text(&outcomes[1]).contains("REGION=fixture"));
    refused(&outcomes[2]);
    refused(&outcomes[3]);
    assert!(!root.path().join("secret-launch-marker").exists());
    assert!(!root.path().join("stolen.txt").exists());
    assert_eq!(
        std::fs::read_to_string(root.path().join("config/.env")).unwrap(),
        CANARY
    );
}

#[tokio::test]
async fn issue522_real_editor_and_shell_refuse_a_template_alias_to_credentials() {
    let root = fixture();
    std::os::unix::fs::symlink(".env", root.path().join("config/.env.sample")).unwrap();
    let outcomes = call_real_tools(
        root.path(),
        vec![
            editor("config/.env.example"),
            editor("config/.env.sample"),
            shell("printf launched > alias-launch-marker; cat config/.env.sample > alias-copy.txt"),
        ],
    )
    .await;
    assert!(text(&outcomes[0]).contains("REGION=fixture"));
    refused(&outcomes[1]);
    refused(&outcomes[2]);
    assert!(!root.path().join("alias-launch-marker").exists());
    assert!(!root.path().join("alias-copy.txt").exists());
    assert_eq!(
        std::fs::read_to_string(root.path().join("config/.env")).unwrap(),
        CANARY
    );
}

#[tokio::test]
async fn issue522_real_tool_dispatch_preserves_an_explicit_project_template_deny() {
    let root = fixture();
    std::fs::write(
        root.path().join(".biorouterignore"),
        "config/.env.example\n",
    )
    .unwrap();
    std::fs::write(root.path().join("config/public.txt"), "safe sibling\n").unwrap();
    let outcomes =
        call_real_tools(
            root.path(),
            vec![
        editor("config/public.txt"),
        editor("config/.env.example"),
        shell("printf launched > policy-launch-marker; cat config/.env.example > policy-copy.txt"),
    ],
        )
        .await;
    assert!(text(&outcomes[0]).contains("safe sibling"));
    refused(&outcomes[1]);
    refused(&outcomes[2]);
    assert!(!root.path().join("policy-launch-marker").exists());
    assert!(!root.path().join("policy-copy.txt").exists());
}

#[tokio::test]
async fn issue522_real_shell_reads_a_template_inside_a_longer_command_line() {
    let root = fixture();
    let outcomes = call_real_tools(
        root.path(),
        vec![
            shell("cat config/.env.example | head -1 > piped-template.txt"),
            shell("printf launched > piped-launch-marker; cat config/.env | head -1 > piped-secret.txt"),
        ],
    )
    .await;
    text(&outcomes[0]);
    assert_eq!(
        std::fs::read(root.path().join("piped-template.txt")).unwrap(),
        b"REGION=fixture\n"
    );
    refused(&outcomes[1]);
    assert!(!root.path().join("piped-launch-marker").exists());
    assert!(!root.path().join("piped-secret.txt").exists());
}

#[tokio::test]
async fn issue546_real_shell_preserves_inert_quoted_heredoc_data_exactly() {
    let root = fixture();
    let body =
        "Use `res$IHS[is.finite(res$IHS)]` when filtering.\nres <- data.frame(IHS = c(1, Inf))\n";
    let command = format!("cat > notes.md <<'EOF'\n{body}EOF\n");
    let outcomes = call_real_tools(root.path(), vec![shell(&command)]).await;
    text(&outcomes[0]);
    assert_eq!(
        std::fs::read_to_string(root.path().join("notes.md")).unwrap(),
        body
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("config/.env")).unwrap(),
        CANARY
    );
}

#[tokio::test]
async fn issue546_real_shell_refuses_executable_stdin_and_a_later_secret_read_before_launch() {
    let root = fixture();
    let outcomes = call_real_tools(root.path(), vec![
        shell("printf launched > executable-launch-marker; sh <<'EOF'\ncat config/.env > executable-copy.txt\nEOF\n"),
        shell("printf launched > later-launch-marker; cat > notes.md <<'EOF'\nordinary prose\nEOF\ncat config/.env > later-copy.txt\n"),
    ]).await;
    for outcome in &outcomes {
        refused(outcome);
    }
    for name in [
        "executable-launch-marker",
        "executable-copy.txt",
        "later-launch-marker",
        "later-copy.txt",
        "notes.md",
    ] {
        assert!(
            !root.path().join(name).exists(),
            "guard must refuse the complete command before launch: {name}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(root.path().join("config/.env")).unwrap(),
        CANARY
    );
}

/// The guard decides where a quoted here-document ends without running it, so
/// the real shell has to end it at the same line for every quoting it accepts:
/// each file below must hold the body exactly, and nothing after it.
#[tokio::test]
async fn issue546_real_shell_ends_every_quoted_delimiter_where_the_guard_does() {
    let root = fixture();
    let body = "res <- data.frame(IHS = c(1, Inf))\nres$IHS[is.finite(res$IHS)]\n";
    let delimiters = ["'EOF'", "\"EOF\"", "\\EOF", "'E'OF"];
    let commands: Vec<String> = delimiters
        .iter()
        .enumerate()
        .map(|(i, delimiter)| format!("cat > out{i}.R <<{delimiter}\n{body}EOF\n"))
        .collect();
    let outcomes = call_real_tools(root.path(), commands.iter().map(|c| shell(c)).collect()).await;
    for (i, outcome) in outcomes.iter().enumerate() {
        text(outcome);
        assert_eq!(
            std::fs::read_to_string(root.path().join(format!("out{i}.R"))).unwrap(),
            body,
            "{}",
            delimiters[i]
        );
    }
}

#[tokio::test]
async fn issue546_real_shell_refuses_code_from_stdin_that_opens_a_secret_by_name() {
    let root = fixture();
    let outcomes = call_real_tools(
        root.path(),
        vec![shell(
            "printf launched > code-launch-marker; python3 - <<'EOF'\nopen('copy.txt', 'w').write(open('config/.env').read())\nEOF\n",
        ), shell(
            "printf launched > bare-launch-marker; cd config && python3 - <<'EOF'\nprint(open('.env').read())\nEOF\n",
        )],
    )
    .await;
    for outcome in &outcomes {
        refused(outcome);
    }
    for name in ["code-launch-marker", "bare-launch-marker", "copy.txt"] {
        assert!(!root.path().join(name).exists(), "{name}");
    }
}

#[tokio::test]
async fn issue538_real_log_read_does_not_authorize_a_sibling_daemon_credential() {
    let root = fixture();
    std::fs::create_dir_all(root.path().join("state/logs")).unwrap();
    std::fs::create_dir_all(root.path().join("state/daemon")).unwrap();
    let payload = "synthetic output\n[shell: command exited with status 0]\n";
    std::fs::write(root.path().join("state/logs/job-fixture.log"), payload).unwrap();
    std::fs::write(
        root.path().join("state/daemon/user-action-key.json"),
        CANARY,
    )
    .unwrap();
    let outcomes = call_real_tools(root.path(), vec![
        shell("cat state/logs/job-fixture.log"),
        editor("state/logs/job-fixture.log"),
        shell("printf launched > state-launch-marker; cat state/daemon/user-action-key.json > state-copy.txt"),
        editor("state/daemon/user-action-key.json"),
    ]).await;
    assert!(text(&outcomes[0]).contains(payload));
    assert!(text(&outcomes[1]).contains("synthetic output"));
    refused(&outcomes[2]);
    refused(&outcomes[3]);
    assert!(!root.path().join("state-launch-marker").exists());
    assert!(!root.path().join("state-copy.txt").exists());
    assert_eq!(
        std::fs::read_to_string(root.path().join("state/daemon/user-action-key.json")).unwrap(),
        CANARY
    );
}
