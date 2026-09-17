use biorouter_mcp::{
    computer_use::{contract, manifest},
    ComputerControllerServer, DeveloperServer, WebDocumentsServer,
};
use rmcp::{
    model::{CallToolRequest, CallToolRequestParams, ClientRequest, Meta},
    ServiceExt,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::Path;

async fn tools_over_mcp<S: rmcp::ServerHandler + Send + 'static>(
    handler: S,
) -> Vec<rmcp::model::Tool> {
    let (client_io, server_io) = tokio::io::duplex(65536);
    let server =
        tokio::spawn(async move { handler.serve(server_io).await.unwrap().waiting().await });
    let client = ().serve(client_io).await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    client.cancel().await.unwrap();
    server.await.unwrap().unwrap();
    tools
}

fn fixture(root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(root.join("ocu"), b"fixture native bytes").unwrap();
    let manifest = json!({"schema_version":1,"upstream_version":manifest::UPSTREAM_VERSION,"upstream_commit":manifest::UPSTREAM_COMMIT,"patch_revision":"biorouter-1","target":manifest::target(),"executable":"ocu","files":[{"path":"ocu","sha256":format!("{:x}", Sha256::digest(b"fixture native bytes"))}]});
    std::fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
}

#[test]
fn packaged_payload_resolves_from_each_layout_and_relocation() {
    let temp = tempfile::tempdir().unwrap();
    for (index, (executable, payload)) in [
        (
            "BioRouter α.app/Contents/Resources/bin/biorouterd",
            "BioRouter α.app/Contents/Resources/computer-use",
        ),
        (
            "BioRouter α.app/Contents/MacOS/biorouter",
            "BioRouter α.app/Contents/Resources/computer-use",
        ),
        (
            "windows/resources/bin/biorouterd.exe",
            "windows/resources/computer-use",
        ),
        (
            "linux/resources/bin/biorouterd",
            "linux/resources/computer-use",
        ),
        ("cli/biorouter", "cli/computer-use"),
    ]
    .into_iter()
    .enumerate()
    {
        let install = temp.path().join(index.to_string());
        let exe = install.join(executable);
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"host").unwrap();
        fixture(&install.join(payload));
        let resolved = manifest::locate_for_executable(&exe).unwrap();
        assert_eq!(
            resolved.executable,
            install.join(payload).join("ocu").canonicalize().unwrap()
        );
        #[cfg(unix)]
        {
            let link = temp.path().join(format!("bin-{index}"));
            std::os::unix::fs::symlink(&exe, &link).unwrap();
            assert_eq!(
                manifest::locate_for_executable(&link).unwrap().executable,
                resolved.executable
            );
        }
    }
}

#[test]
fn copied_cli_follows_install_origin_and_keeps_local_payload_precedence() {
    let temp = tempfile::tempdir().unwrap();
    let resources = temp.path().join("Application α/resources");
    let source_bin = resources.join("bin");
    std::fs::create_dir_all(&source_bin).unwrap();
    let payload = resources.join("computer-use");
    fixture(&payload);
    let install = temp.path().join("Local/Biorouter/bin");
    std::fs::create_dir_all(&install).unwrap();
    let exe = install.join("biorouter.exe");
    std::fs::write(&exe, b"copied CLI").unwrap();
    std::fs::write(
        install.join(".biorouter-origin"),
        format!("\u{feff} {} \nignored second line", source_bin.display()),
    )
    .unwrap();
    let resolved = manifest::locate_for_executable(&exe).unwrap();
    assert_eq!(resolved.root, payload.canonicalize().unwrap());
    assert!(!resolved.development_override);
    std::fs::write(payload.join("ocu"), b"corrupt origin payload").unwrap();
    assert!(manifest::locate_for_executable(&exe)
        .unwrap_err()
        .to_string()
        .contains("checksum"));
    let local = install.join("computer-use");
    fixture(&local);
    assert_eq!(
        manifest::locate_for_executable(&exe).unwrap().root,
        local.canonicalize().unwrap()
    );
}

#[test]
fn copied_cli_ignores_missing_invalid_and_stale_install_origins() {
    let temp = tempfile::tempdir().unwrap();
    let exe = temp.path().join("biorouter.exe");
    std::fs::write(&exe, b"copied CLI").unwrap();
    assert!(manifest::locate_for_executable(&exe).is_err());
    for raw in [
        Vec::new(),
        b"  \n".to_vec(),
        b"relative/resources/bin".to_vec(),
        b"../resources/bin".to_vec(),
        vec![0xff, 0xfe],
        temp.path()
            .join("deleted/resources/bin")
            .to_string_lossy()
            .as_bytes()
            .to_vec(),
    ] {
        std::fs::write(temp.path().join(".biorouter-origin"), &raw).unwrap();
        assert!(
            manifest::locate_for_executable(&exe).is_err(),
            "unexpected origin: {raw:?}"
        );
    }
}

#[test]
fn payload_rejects_corruption_wrong_pin_and_escaping_paths() {
    let temp = tempfile::tempdir().unwrap();
    fixture(temp.path());
    assert!(manifest::validate(temp.path(), false).is_ok());
    std::fs::write(temp.path().join("ocu"), b"tampered").unwrap();
    assert!(manifest::validate(temp.path(), false)
        .unwrap_err()
        .to_string()
        .contains("checksum"));
    fixture(temp.path());
    let path = temp.path().join("manifest.json");
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for (field, value) in [
        ("upstream_version", "999"),
        ("upstream_commit", "unreviewed"),
        ("target", "foreign"),
        ("executable", "../ocu"),
    ] {
        let mut wrong = original.clone();
        wrong[field] = json!(value);
        std::fs::write(&path, serde_json::to_vec(&wrong).unwrap()).unwrap();
        assert!(
            manifest::validate(temp.path(), false).is_err(),
            "must reject {field}"
        );
    }
}

#[tokio::test]
async fn capabilities_are_disjoint_and_listing_never_launches_a_helper() {
    let (client_io, server_io) = tokio::io::duplex(65536);
    let server = tokio::spawn(async move {
        ComputerControllerServer::new()
            .serve(server_io)
            .await
            .unwrap()
            .waiting()
            .await
    });
    let client = ().serve(client_io).await.unwrap();
    let roster = client.list_all_tools().await.unwrap();
    assert_eq!(roster.len(), 10);
    for expected in contract::TOOL_NAMES {
        assert!(roster.iter().any(|tool| tool.name == expected));
    }
    for retired in [
        "automation_script",
        "computer_control",
        "web_scrape",
        "cache",
    ] {
        let error = client
            .call_tool(CallToolRequestParams {
                meta: None,
                name: retired.into(),
                arguments: Some(Default::default()),
                task: None,
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Unknown Computer Use tool"));
    }
    let missing = client
        .call_tool(CallToolRequestParams {
            meta: None,
            name: "list_apps".into(),
            arguments: None,
            task: None,
        })
        .await
        .unwrap_err();
    assert!(missing.to_string().contains("approval_required"));
    let invalid = client
        .call_tool(CallToolRequestParams {
            meta: None,
            name: "click".into(),
            arguments: Some(json!({"app":42}).as_object().unwrap().clone()),
            task: None,
        })
        .await
        .unwrap_err();
    assert!(invalid.to_string().contains("schema"), "{invalid:?}");
    for (session, expected) in [
        ("contract-chat", "computer_use_stale_state"),
        ("different-chat", "computer_use_session_mismatch"),
    ] {
        let mut extensions = rmcp::model::Extensions::default();
        extensions.insert(Meta(
            json!({"biorouter-session-id":session, "computer_use_generation":"grant", "progressToken":"contract-progress"})
                .as_object().unwrap().clone(),
        ));
        let request = CallToolRequest {
            method: Default::default(),
            params: CallToolRequestParams {
                meta: None,
                name: "click".into(),
                arguments: Some(
                    json!({"app":"fixture", "element_index":"0"})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
                task: None,
            },
            extensions,
        };
        let error = client
            .send_request(ClientRequest::CallToolRequest(request))
            .await
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{error:?}");
    }
    client.cancel().await.unwrap();
    server.await.unwrap().unwrap();

    let utilities = tools_over_mcp(WebDocumentsServer::new()).await;
    assert_eq!(utilities.len(), 5);
    for name in ["web_scrape", "xlsx_tool", "docx_tool", "pdf_tool", "cache"] {
        assert!(utilities.iter().any(|tool| tool.name == name));
    }
    let developer = tools_over_mcp(DeveloperServer::new()).await;
    for removed in [
        "screen_capture",
        "list_windows",
        "automation_script",
        "computer_control",
    ] {
        assert!(developer.iter().all(|tool| tool.name != removed));
    }
}
