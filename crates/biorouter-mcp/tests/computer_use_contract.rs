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

#[test]
fn native_numeric_contract_rejects_invalid_supplied_values() {
    for (name, argument, valid, invalid) in [
        (
            "click",
            "click_count",
            vec![json!(1), json!(2), json!(100)],
            vec![
                json!(0),
                json!(-1),
                json!(101),
                json!(1.5),
                json!(1e100),
                json!(true),
                json!(null),
                json!("2"),
            ],
        ),
        (
            "scroll",
            "pages",
            vec![json!(0.25), json!(1), json!(100)],
            vec![
                json!(0),
                json!(-1),
                json!(100.01),
                json!(1e100),
                json!(true),
                json!(null),
                json!("2"),
            ],
        ),
    ] {
        let tool = contract::tools()
            .into_iter()
            .find(|tool| tool.name == name)
            .unwrap();
        let schema = serde_json::Value::Object((*tool.input_schema).clone());
        let validator = jsonschema::validator_for(&schema).unwrap();
        let mut arguments = if name == "scroll" {
            json!({"app":"synthetic", "element_index":"1", "direction":"down"})
        } else {
            json!({"app":"synthetic"})
        };
        assert!(
            validator.is_valid(&arguments),
            "absent optional value retains its default"
        );
        for value in valid {
            arguments[argument] = value;
            assert!(validator.is_valid(&arguments), "{arguments}");
        }
        for value in invalid {
            arguments[argument] = value;
            assert!(!validator.is_valid(&arguments), "{arguments}");
        }
    }
}

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
    let manifest = json!({"schema_version":1,"upstream_version":manifest::UPSTREAM_VERSION,"upstream_commit":manifest::UPSTREAM_COMMIT,"patch_revision":manifest::patch_revision(),"target":manifest::target(),"executable":"ocu","files":[{"path":"ocu","sha256":format!("{:x}", Sha256::digest(b"fixture native bytes"))}]});
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
        ("patch_revision", "outdated"),
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
        assert!(error.to_string().contains("Unknown Biorouter Copilot tool"));
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

/// Every `server__tool` a SHIPPED skill names must be a tool that server really
/// advertises.
///
/// ⚠ This is not hygiene. A skill's prose is injected into the model's context
/// as instructions, so a name that no longer resolves does not fail loudly — the
/// model tries the call, gets "unknown tool", and silently falls back to the
/// slower path the skill describes as a last resort. Nothing in CI noticed.
///
/// It had already happened when this test was written: the three office skills
/// shipped on 2026-09-18 told the model to call
/// `computercontroller__{docx,xlsx,pdf}_tool`, and those three tools had moved
/// to the `webdocuments` server in this branch. The skills were right when they
/// were written and wrong when they shipped, which is exactly the drift a
/// cross-crate gate exists to catch: the skills live in `biorouter`, the tools
/// in `biorouter-mcp`, and neither crate's own tests can see both halves.
#[tokio::test]
async fn every_tool_a_shipped_skill_names_is_a_tool_that_exists() {
    let mut registry: Vec<(&str, Vec<String>)> = vec![
        (
            "computercontroller",
            contract::tools()
                .iter()
                .map(|tool| tool.name.to_string())
                .collect(),
        ),
        (
            "webdocuments",
            tools_over_mcp(WebDocumentsServer::new())
                .await
                .iter()
                .map(|tool| tool.name.to_string())
                .collect(),
        ),
        (
            "developer",
            tools_over_mcp(DeveloperServer::new())
                .await
                .iter()
                .map(|tool| tool.name.to_string())
                .collect(),
        ),
    ];
    registry.sort();

    let skills = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../biorouter/src/agents/builtin_skills")
        .canonicalize()
        .expect("builtin skills directory");
    let mut checked = 0usize;
    let mut unknown: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&skills).expect("read builtin skills") {
        let skill = entry.expect("skill entry").path().join("SKILL.md");
        let Ok(body) = std::fs::read_to_string(&skill) else {
            continue;
        };
        // Only backtick-quoted references: prose mentions a server by name all
        // the time, and matching those would make the gate noisy enough to be
        // switched off.
        for quoted in body.split('`').skip(1).step_by(2) {
            let Some((server, tool)) = quoted.split_once("__") else {
                continue;
            };
            if tool.is_empty() || !tool.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                continue;
            }
            let Some((_, tools)) = registry.iter().find(|(name, _)| *name == server) else {
                continue;
            };
            checked += 1;
            if !tools.iter().any(|name| name == tool) {
                unknown.push(format!(
                    "{}: `{quoted}` -- `{server}` advertises {tools:?}",
                    skill.strip_prefix(&skills).unwrap_or(&skill).display()
                ));
            }
        }
    }
    assert!(
        unknown.is_empty(),
        "shipped skills name {} tool(s) that do not exist:\n  {}",
        unknown.len(),
        unknown.join("\n  ")
    );
    // A gate that checked nothing would pass just as quietly as one that passed.
    assert!(
        checked >= 3,
        "expected the office skills' tool references to be checked, saw {checked}"
    );
}

/// The readiness probe's bound must exceed every bound it supervises.
///
/// ⚠ This is a rule about NESTING, not about a number. `Runtime::doctor` wraps a
/// helper that imposes its own deadline and answers honestly when it expires —
/// the Windows helper runs PowerShell under `context.WithTimeout(30s)` and
/// reports `missing_dependency`, "Windows runtime timed out after 30s". An outer
/// bound equal to the inner one always wins, because it starts strictly earlier
/// (spawn, process start and the helper's own setup all precede the inner
/// clock), so the inner deadline becomes dead code and its diagnosis is never
/// delivered. Both were 30 s, and a Windows runner reported only "could not
/// check the native runtime" for it.
///
/// The bounds are read out of the two sources rather than restated, because a
/// test carrying its own copy of a number passes while the real one drifts.
#[test]
fn the_probe_bound_exceeds_every_helper_bound_it_supervises() {
    let runtime = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/computer_use/runtime.rs"),
    )
    .expect("runtime.rs");
    let outer: u64 = runtime
        .split_once("const PROBE_BOUND:")
        .and_then(|(_, rest)| rest.split_once("from_secs("))
        .and_then(|(_, rest)| rest.split_once(')'))
        .and_then(|(value, _)| value.trim().parse().ok())
        .expect("PROBE_BOUND must be a literal `from_secs(N)` this test can read");

    // The helper sources are fetched at build time, so read them only when a
    // checkout is present; the rule is still pinned wherever one is.
    let checkout =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/computer-use/source.noindex/apps");
    let mut checked = 0usize;
    for app in ["OpenComputerUseWindows", "OpenComputerUseLinux"] {
        let Ok(source) = std::fs::read_to_string(checkout.join(app).join("main.go")) else {
            continue;
        };
        // `split` rather than `match_indices` + slicing: clippy::string_slice
        // rejects indexing a `str`, because a byte index that is not a character
        // boundary panics. The indices here happen to be safe (they come from
        // `match_indices`), but the lint is denied workspace-wide and the split
        // form needs no such reasoning to read.
        for tail in source
            .split("context.WithTimeout(context.Background(), ")
            .skip(1)
        {
            // The split pattern already consumed up to `Background(), `, so the
            // duration is at the head of `tail`. The old `match_indices` form
            // kept the pattern and had to skip past it with `split_once("), ")`.
            let seconds: u64 = tail
                .split_once("*time.Second")
                .and_then(|(value, _)| value.trim().parse().ok())
                .expect("a helper bound this test can read");
            checked += 1;
            assert!(
                outer > seconds,
                "{app} bounds its bridge at {seconds}s and the probe supervising it allows \
                 {outer}s. An outer bound that does not EXCEED the inner one always wins — it \
                 starts earlier — so the helper's own diagnosis can never be delivered."
            );
        }
    }
    if checked == 0 {
        // Say so rather than passing quietly: with no checkout this asserted nothing.
        eprintln!("no helper checkout present; the nesting rule was not exercised");
    }
}
