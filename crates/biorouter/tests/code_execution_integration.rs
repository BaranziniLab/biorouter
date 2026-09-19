//! Live end-to-end tests for the `code_execution` platform extension.
//!
//! Unlike the unit tests in `code_execution_extension.rs` (which exercise the JS
//! engine in isolation), these wire up a *real* `ExtensionManager` with the
//! bundled `developer` extension and drive `execute_code` end-to-end through
//! `dispatch_tool_call`, the same path the agent loop uses. Each test models a
//! realistic use case, from trivial arithmetic up to multi-tool data-flow chains.

use std::sync::Arc;

use rmcp::model::{CallToolRequestParams, RawContent};
use rmcp::object;
use serde_json::json;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

use biorouter::agents::extension::ExtensionConfig;
use biorouter::agents::extension_manager::ExtensionManager;

const SESSION: &str = "code-exec-it";

/// Build an ExtensionManager with `developer` + `code_execution` enabled.
async fn manager() -> Arc<ExtensionManager> {
    let temp_dir = tempfile::tempdir().unwrap();
    // Keep the tempdir alive for the process lifetime; tests are short-lived.
    let temp_path = temp_dir.keep();
    let session_manager = Arc::new(biorouter::session::SessionManager::new(temp_path));
    let manager = Arc::new(ExtensionManager::new(
        Arc::new(Mutex::new(None)),
        session_manager,
    ));

    manager
        .add_extension(ExtensionConfig::Builtin {
            name: "developer".to_string(),
            description: "developer".to_string(),
            display_name: Some("Developer".to_string()),
            timeout: Some(300),
            bundled: Some(true),
            available_tools: vec![],
        })
        .await
        .expect("add developer");

    manager
        .add_extension(ExtensionConfig::Platform {
            name: "code_execution".to_string(),
            description: "Execute JavaScript code in a sandboxed environment".to_string(),
            bundled: Some(true),
            available_tools: vec![],
        })
        .await
        .expect("add code_execution");

    manager
}

async fn manager_with_webdocuments() -> Arc<ExtensionManager> {
    let manager = manager().await;
    manager
        .add_extension(ExtensionConfig::Builtin {
            name: "webdocuments".to_string(),
            description: "Web and document tools".to_string(),
            display_name: Some("Web & Documents".to_string()),
            timeout: Some(300),
            bundled: Some(true),
            available_tools: vec![],
        })
        .await
        .expect("add webdocuments");
    manager
}

async fn manager_with_workspace() -> Arc<ExtensionManager> {
    let manager = manager().await;
    manager
        .add_extension(ExtensionConfig::Platform {
            name: "workspace".to_string(),
            description: "Workspace Control".to_string(),
            bundled: Some(true),
            available_tools: vec![],
        })
        .await
        .expect("add workspace");
    manager
}

/// Run `execute_code` with the given JS source and return the textual result
/// (the `Result: ...` string the agent would see). Panics on dispatch errors.
async fn exec(manager: &Arc<ExtensionManager>, code: &str) -> String {
    let call = CallToolRequestParams {
        task: None,
        meta: None,
        name: "code_execution__execute_code".into(),
        arguments: Some(object!({ "code": code })),
    };
    let dispatched = manager
        .dispatch_tool_call(
            SESSION,
            call,
            biorouter::privacy::CallCapability::public_enforced(),
            CancellationToken::new(),
        )
        .await
        .expect("dispatch");
    let result = dispatched.result.await.expect("tool result");
    assert!(
        !result.is_error.unwrap_or(false),
        "execute_code reported is_error=true: {:?}",
        result.content
    );
    // Concatenate all text content blocks.
    result
        .content
        .iter()
        .filter_map(|c| match &c.raw {
            RawContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run `execute_code` and return `(is_error, joined_text)` without asserting.
/// Used for cases where an error result is the expected outcome.
async fn exec_raw(manager: &Arc<ExtensionManager>, code: &str) -> (bool, String) {
    let call = CallToolRequestParams {
        task: None,
        meta: None,
        name: "code_execution__execute_code".into(),
        arguments: Some(object!({ "code": code })),
    };
    let dispatched = manager
        .dispatch_tool_call(
            SESSION,
            call,
            biorouter::privacy::CallCapability::public_enforced(),
            CancellationToken::new(),
        )
        .await
        .expect("dispatch");
    let result = dispatched.result.await.expect("tool result");
    let text = result
        .content
        .iter()
        .filter_map(|c| match &c.raw {
            RawContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    (result.is_error.unwrap_or(false), text)
}

/// Create a temp dir *inside* the current working directory (the developer
/// extension refuses paths outside the working dir) and return its canonical
/// absolute path as a String.
fn workdir_tempdir() -> (tempfile::TempDir, String) {
    let dir = tempfile::Builder::new()
        .prefix("codeexec_it")
        .tempdir_in(".")
        .unwrap();
    let abs = dir
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    (dir, abs)
}

/// Same as `exec` but for read_module / search_modules etc.
async fn call_tool(manager: &Arc<ExtensionManager>, tool: &str, args: serde_json::Value) -> String {
    let call = CallToolRequestParams {
        task: None,
        meta: None,
        name: format!("code_execution__{tool}").into(),
        arguments: Some(args.as_object().unwrap().clone()),
    };
    let dispatched = manager
        .dispatch_tool_call(
            SESSION,
            call,
            biorouter::privacy::CallCapability::public_enforced(),
            CancellationToken::new(),
        )
        .await
        .expect("dispatch");
    let result = dispatched.result.await.expect("tool result");
    result
        .content
        .iter()
        .filter_map(|c| match &c.raw {
            RawContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// 1-4: Pure JS (no tool calls) — exercises the engine + record_result paths.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn case01_arithmetic() {
    let m = manager().await;
    assert_eq!(exec(&m, "record_result(2 + 2 * 10)").await, "Result: 22");
}

#[tokio::test]
async fn case02_string_methods() {
    let m = manager().await;
    assert_eq!(
        exec(&m, r#"record_result("biorouter".toUpperCase())"#).await,
        "Result: \"BIOROUTER\""
    );
}

#[tokio::test]
async fn case03_array_pipeline() {
    let m = manager().await;
    // map/filter/reduce chain
    let out = exec(
        &m,
        r#"
        const xs = [1,2,3,4,5,6,7,8,9,10];
        const result = xs.filter(x => x % 2 === 0).map(x => x * x).reduce((a,b) => a+b, 0);
        record_result(result);
        "#,
    )
    .await;
    assert_eq!(out, "Result: 220"); // 4+16+36+64+100
}

#[tokio::test]
async fn case04_object_result_is_valid_json() {
    let m = manager().await;
    let out = exec(
        &m,
        r#"record_result({ name: "x", values: [1,2,3], nested: { ok: true } })"#,
    )
    .await;
    let json_str = out.strip_prefix("Result: ").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(json_str).expect("valid JSON");
    assert_eq!(parsed["values"].as_array().unwrap().len(), 3);
    assert_eq!(parsed["nested"]["ok"], true);
}

// ---------------------------------------------------------------------------
// 5-7: Single tool calls into the developer extension.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn case05_shell_echo() {
    let m = manager().await;
    let out = exec(
        &m,
        r#"
        import { shell } from "developer";
        const r = shell({ command: "echo hello-from-shell" });
        record_result(r);
        "#,
    )
    .await;
    assert!(out.contains("hello-from-shell"), "got: {out}");
}

#[tokio::test]
async fn case06_shell_numeric_output_processed_in_js() {
    let m = manager().await;
    // Shell prints a number; JS adds to it. Verifies result parsing + arithmetic.
    let out = exec(
        &m,
        r#"
        import { shell } from "developer";
        const r = shell({ command: "echo 40" });
        // r may be parsed as a number or come back as text; coerce.
        record_result(Number(String(r).trim()) + 2);
        "#,
    )
    .await;
    let n: f64 = out.strip_prefix("Result: ").unwrap().parse().unwrap();
    assert_eq!(n, 42.0, "got: {out}");
}

#[tokio::test]
async fn case07_shell_multiline_split_and_count() {
    let m = manager().await;
    let out = exec(
        &m,
        r#"
        import { shell } from "developer";
        const r = String(shell({ command: "echo a; echo b; echo c" }));
        const lines = r.split("\n").filter(l => l.length > 0);
        record_result({ count: lines.length, first: lines[0] });
        "#,
    )
    .await;
    let json_str = out.strip_prefix("Result: ").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(json_str).unwrap();
    assert_eq!(parsed["count"], 3);
    assert_eq!(parsed["first"], "a");
}

// ---------------------------------------------------------------------------
// 8-11: text_editor + multi-tool chains (the headline "batch in one call" use).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn case08_write_then_view_file() {
    let m = manager().await;
    let (_dir, dir_s) = workdir_tempdir();
    let path_s = format!("{dir_s}/note.txt");
    let out = exec(
        &m,
        &format!(
            r#"
            import {{ text_editor }} from "developer";
            text_editor({{ command: "write", path: "{path_s}", file_text: "line one\nline two\n" }});
            const content = text_editor({{ command: "view", path: "{path_s}" }});
            record_result(content);
            "#
        ),
    )
    .await;
    assert!(out.contains("line one"), "got: {out}");
    assert!(out.contains("line two"), "got: {out}");
    // File really exists on disk with expected content.
    let on_disk = std::fs::read_to_string(&path_s).unwrap();
    assert_eq!(on_disk, "line one\nline two\n");
}

#[tokio::test]
async fn case09_shell_write_texteditor_read_dataflow() {
    let m = manager().await;
    let (_dir, dir_s) = workdir_tempdir();
    let path_s = format!("{dir_s}/data.txt");
    // shell writes the file; text_editor reads it back; JS asserts on content.
    let out = exec(
        &m,
        &format!(
            r#"
            import {{ shell, text_editor }} from "developer";
            shell({{ command: "echo 'generated content' > {path_s}" }});
            const view = String(text_editor({{ command: "view", path: "{path_s}" }}));
            record_result({{ hasContent: view.includes("generated content") }});
            "#
        ),
    )
    .await;
    let json_str = out.strip_prefix("Result: ").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(json_str).unwrap();
    assert_eq!(parsed["hasContent"], true);
}

#[tokio::test]
async fn case10_loop_multiple_tool_calls() {
    let m = manager().await;
    // A JS loop issuing several synchronous tool calls and aggregating.
    let out = exec(
        &m,
        r#"
        import { shell } from "developer";
        let total = 0;
        for (let i = 1; i <= 5; i++) {
            const r = String(shell({ command: "echo " + i })).trim();
            total += Number(r);
        }
        record_result(total);
        "#,
    )
    .await;
    let n: f64 = out.strip_prefix("Result: ").unwrap().parse().unwrap();
    assert_eq!(n, 15.0, "got: {out}");
}

#[tokio::test]
async fn case11_write_edit_view_chain() {
    let m = manager().await;
    let (_dir, dir_s) = workdir_tempdir();
    let path_s = format!("{dir_s}/doc.md");
    let out = exec(
        &m,
        &format!(
            r##"
            import {{ text_editor }} from "developer";
            text_editor({{ command: "write", path: "{path_s}", file_text: "# Title\nbody\n" }});
            text_editor({{ command: "str_replace", path: "{path_s}", old_str: "# Title", new_str: "# Changed" }});
            const out = String(text_editor({{ command: "view", path: "{path_s}" }}));
            record_result({{ changed: out.includes("# Changed"), gone: !out.includes("# Title") }});
            "##
        ),
    )
    .await;
    let json_str = out.strip_prefix("Result: ").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(json_str).unwrap();
    assert_eq!(parsed["changed"], true);
    assert_eq!(parsed["gone"], true);
}

// ---------------------------------------------------------------------------
// 12-14: JSON handling, namespace imports, JS built-ins.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn case12_json_parse_stringify() {
    let m = manager().await;
    let out = exec(
        &m,
        r#"
        const obj = JSON.parse('{"a": 1, "b": [2,3]}');
        record_result({ sum: obj.a + obj.b[0] + obj.b[1] });
        "#,
    )
    .await;
    let json_str = out.strip_prefix("Result: ").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(json_str).unwrap();
    assert_eq!(parsed["sum"], 6);
}

#[tokio::test]
async fn case13_namespace_import() {
    let m = manager().await;
    let out = exec(
        &m,
        r#"
        import * as developer from "developer";
        const r = developer.shell({ command: "echo namespaced" });
        record_result(String(r).includes("namespaced"));
        "#,
    )
    .await;
    assert_eq!(out, "Result: true");
}

#[tokio::test]
async fn case14_js_builtins_math_json() {
    let m = manager().await;
    let out = exec(
        &m,
        r#"
        const data = [3.2, 1.5, 9.8, 4.1];
        const max = Math.max(...data);
        const rounded = data.map(x => Math.round(x));
        record_result({ max, rounded, sorted: [...data].sort((a,b)=>a-b) });
        "#,
    )
    .await;
    let json_str = out.strip_prefix("Result: ").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(json_str).unwrap();
    assert_eq!(parsed["max"], 9.8);
    assert_eq!(parsed["rounded"][2], 10);
}

// ---------------------------------------------------------------------------
// 15-17: Error handling & edge cases.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn case15_tool_error_propagates_as_js_exception() {
    let m = manager().await;
    // cat a nonexistent file: developer shell should fail; the JS native fn
    // throws, the module rejects, and execute_code returns a "Module error".
    //
    // `exec_raw`, not `exec`: an error result IS the expected outcome here. Since
    // PAR-02 the shell reports a non-zero exit as `is_error`, so the failure now
    // propagates all the way out as `execute_code`'s own error flag — which is
    // the point of the case. (`case16` covers the other side: an error the JS
    // catches leaves `execute_code` successful.)
    let (is_error, out) = exec_raw(
        &m,
        r#"
        import { shell } from "developer";
        const r = shell({ command: "cat /nonexistent/path/xyz123" });
        record_result(r);
        "#,
    )
    .await;
    // What we must NOT see is a silent success that pretends the file was read.
    assert!(
        is_error,
        "an uncaught tool failure must surface as execute_code's error flag, \
         not as a successful result: {out}"
    );
    assert!(
        out.contains("Module error")
            || out.to_lowercase().contains("no such file")
            || out.to_lowercase().contains("error"),
        "expected an error surfaced, got: {out}"
    );
    // The shell's exit status reaches the JS layer, so the failure is legible
    // rather than an empty rejection.
    assert!(
        out.contains("exited with status 1") || out.to_lowercase().contains("no such file"),
        "the propagated error must name what actually failed, got: {out}"
    );
}

#[tokio::test]
async fn case16_try_catch_recovers_from_tool_error() {
    let m = manager().await;
    let out = exec(
        &m,
        r#"
        import { shell } from "developer";
        let captured = "none";
        try {
            shell({ command: "cat /nonexistent/path/xyz123" });
        } catch (e) {
            captured = "caught";
        }
        record_result(captured);
        "#,
    )
    .await;
    // If shell errors raise a JS exception, try/catch should recover.
    // If shell returns an error string (no throw), captured stays "none".
    assert!(
        out == "Result: \"caught\"" || out == "Result: \"none\"",
        "got: {out}"
    );
}

#[tokio::test]
async fn case17_no_record_result_returns_undefined() {
    let m = manager().await;
    let out = exec(&m, r#"const x = 1 + 1;"#).await;
    assert_eq!(out, "Result: undefined");
}

// ---------------------------------------------------------------------------
// 18-20: Discovery tools (read_module / search_modules) + syntax errors.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn case18_read_module_lists_developer_tools() {
    let m = manager().await;
    let out = call_tool(&m, "read_module", json!({ "module_path": "developer" })).await;
    assert!(out.contains("shell"), "got: {out}");
    assert!(out.contains("text_editor"), "got: {out}");
    assert!(
        out.contains("import * as developer"),
        "should include import hint, got: {out}"
    );
}

#[tokio::test]
async fn case19_search_modules_finds_shell() {
    let m = manager().await;
    let out = call_tool(&m, "search_modules", json!({ "terms": "shell" })).await;
    assert!(out.contains("developer/shell"), "got: {out}");
    assert!(out.contains("import * as module_developer"), "got: {out}");
    assert!(out.contains("command: string"), "got: {out}");
    assert!(!out.contains("Use the read_module"), "got: {out}");
}

#[tokio::test]
async fn agent_loop_subagent_is_not_importable_through_code_execution() {
    let manager = manager_with_workspace().await;
    let direct_tools = manager
        .get_prefixed_tools(Some("workspace".to_string()))
        .await
        .expect("list workspace tools");
    assert!(
        direct_tools
            .iter()
            .any(|tool| tool.name.as_ref() == "workspace__subagent"),
        "positive control: delegation must remain available as a direct agent-loop tool"
    );

    let module = call_tool(
        &manager,
        "read_module",
        json!({ "module_path": "workspace" }),
    )
    .await;
    assert!(
        module.contains("workspace_list"),
        "positive control: the workspace module must remain discoverable: {module}"
    );
    assert!(
        !module.contains("[\"subagent\"]"),
        "an agent-loop-only tool must not be advertised as a callable module function: {module}"
    );

    let search = call_tool(
        &manager,
        "search_modules",
        json!({ "terms": ["subagent", "delegate"] }),
    )
    .await;
    assert!(
        !search.contains("workspace/subagent"),
        "search_modules must not direct execute_code to the non-callable extension path: {search}"
    );

    let (is_error, output) = exec_raw(
        &manager,
        r#"
        import * as workspace from "workspace";
        record_result(workspace["subagent"]({instructions: "check delegation"}));
        "#,
    )
    .await;
    assert!(
        is_error,
        "the agent-loop-only tool must not be callable from code"
    );
    assert!(
        !output.contains("dispatched by the agent loop"),
        "the code bridge must reject the call before it reaches the extension stub: {output}"
    );
}

/// Issue #26: a search that matches nothing is an EMPTY RESULT, not a broken
/// tool — it must come back is_error=false with guidance, instead of the old
/// `[tool_error kind=tool_failure retryable=false] No matches found for: …`
/// that read as tool breakage and fed the failure-streak counters.
#[tokio::test]
async fn search_modules_no_match_is_a_success_with_guidance() {
    let m = manager().await;
    let call = CallToolRequestParams {
        task: None,
        meta: None,
        name: "code_execution__search_modules".into(),
        arguments: Some(object!({
            "terms": ["create skill", "make skill", "skill maker", "draft skill"]
        })),
    };
    let dispatched = m
        .dispatch_tool_call(
            SESSION,
            call,
            biorouter::privacy::CallCapability::public_enforced(),
            CancellationToken::new(),
        )
        .await
        .expect("dispatch");
    let result = dispatched.result.await.expect("tool result");
    let text = result
        .content
        .iter()
        .filter_map(|c| match &c.raw {
            RawContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !result.is_error.unwrap_or(false),
        "a no-match search must not be a tool error, got: {text}"
    );
    assert!(text.contains("No tools matched"), "got: {text}");
    assert!(text.contains("installed MCP tools"), "got: {text}");
}

#[tokio::test]
async fn simple_news_search_discovery_and_fetch_needs_no_read_module_call() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "<rss><channel><item><title>Apple Watch update</title><link>https://example.test/watch</link></item></channel></rss>",
        ))
        .mount(&mock_server)
        .await;
    let manager = manager_with_webdocuments().await;

    let discovery = call_tool(
        &manager,
        "search_modules",
        json!({ "terms": ["web", "search", "browser", "news"] }),
    )
    .await;
    assert!(discovery.contains("webdocuments/web_scrape"));
    assert!(discovery.contains("module_webdocuments[\"web_scrape\"]"));
    assert!(discovery.contains("do not call read_module"));

    let result = exec(
        &manager,
        &format!(
            r#"
            import * as module_webdocuments from "webdocuments";
            const feed = module_webdocuments["web_scrape"]({{ url: "{}" }});
            record_result(feed);
            "#,
            mock_server.uri()
        ),
    )
    .await;
    assert!(result.contains("Apple Watch update"), "got: {result}");
}

#[tokio::test]
async fn failed_nested_web_fetch_is_an_execute_code_error() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503).set_body_string("search unavailable"))
        .mount(&mock_server)
        .await;
    let manager = manager_with_webdocuments().await;
    let (is_error, result) = exec_raw(
        &manager,
        &format!(
            r#"
            import * as webdocuments from "webdocuments";
            record_result(webdocuments["web_scrape"]({{ url: "{}" }}));
            "#,
            mock_server.uri()
        ),
    )
    .await;
    assert!(
        is_error,
        "failed inner fetch must fail execute_code: {result}"
    );
    assert!(result.contains("503"), "got: {result}");
}

#[tokio::test]
async fn case20_syntax_error_is_reported() {
    let m = manager().await;
    let (is_error, out) = exec_raw(&m, r#"record_result( this is not valid js )"#).await;
    // A malformed script must surface as an error result, not a silent success.
    assert!(
        is_error,
        "syntax error should set is_error=true, got: {out}"
    );
    assert!(
        out.to_lowercase().contains("parse error") || out.to_lowercase().contains("syntaxerror"),
        "expected a parse error, got: {out}"
    );
    // An ordinary typo carries NO embedded-payload hint (issue #23) — the hint
    // must not fire on parse errors unrelated to templates.
    assert!(
        !out.contains("String.raw does NOT make"),
        "unrelated parse error must not get the template hint, got: {out}"
    );
}

// ---- issue #23: the six transcript parse-failure shapes now self-correct ----
//
// Each payload below reproduces one row of issue #23's failure table (session
// "starsign master conversation"): model-generated scripts embedding shell,
// CSS/markdown, or prose in template literals, mostly behind `String.raw` in
// the belief that it neutralises `${…}` and backticks. It does not — the code
// is genuinely invalid JS — so the fix is a parse-error annotation teaching the
// escape (`${"$"}{`) and the plain-string / write-to-file recoveries.

/// Assert the exec result is a parse error carrying the #23 recovery hint.
fn assert_parse_error_with_hint(is_error: bool, out: &str) {
    assert!(is_error, "embedded payload must fail the parse, got: {out}");
    assert!(out.contains("Parse error"), "got: {out}");
    assert!(
        out.contains("String.raw does NOT make ${…} literal"),
        "parse error must carry the self-correction hint, got: {out}"
    );
    assert!(
        out.contains(r#"${"$"}{VAR}"#),
        "hint must teach the dollar-brace escape, got: {out}"
    );
    assert!(
        out.contains("developer/text_editor") && out.contains("developer/shell"),
        "hint must offer the write-to-file recovery, got: {out}"
    );
}

/// Row 1: bash indirect expansion `"${!v:-}"` inside a `String.raw` shell loop
/// ("expected token '}', got ':' in template literal").
#[tokio::test]
async fn parse_hint_bash_indirect_expansion_in_string_raw() {
    let m = manager().await;
    let (is_error, out) = exec_raw(
        &m,
        "const s = String.raw`for v in A B; do echo \"${!v:-}\"; done`;\nrecord_result(s);",
    )
    .await;
    assert_parse_error_with_hint(is_error, &out);
}

/// Row 2: CSS `@media (prefers-color-scheme: dark)` landing in object-literal
/// position after a markdown code fence's backticks ended the template early.
#[tokio::test]
async fn parse_hint_css_media_query_after_fence_breakout() {
    let m = manager().await;
    let (is_error, out) = exec_raw(
        &m,
        "const doc = String.raw`# Theme\n```css\n@media (prefers-color-scheme: dark) { body { background: black } }\n```\n`;\nrecord_result(doc);",
    )
    .await;
    assert_parse_error_with_hint(is_error, &out);
}

/// Row 3: multi-line Python heredoc whose embedded quotes/backticks unbalance
/// the literal ("unterminated string literal").
#[tokio::test]
async fn parse_hint_python_heredoc_with_embedded_quotes() {
    let m = manager().await;
    let (is_error, out) = exec_raw(
        &m,
        "import { shell } from \"developer\";\nconst out = shell({ command: String.raw`python3 - <<'PY'\nprint(\"it's `quoted`\")\nPY` });\nrecord_result(out);",
    )
    .await;
    assert_parse_error_with_hint(is_error, &out);
}

/// Row 4: same class — shell heredoc using `${VAR:-default}` parameter
/// expansion inside `String.raw`.
#[tokio::test]
async fn parse_hint_shell_heredoc_with_default_expansion() {
    let m = manager().await;
    let (is_error, out) = exec_raw(
        &m,
        "const cmd = String.raw`cat <<EOF\nprefix=${PREFIX:-/usr/local}\nEOF`;\nrecord_result(cmd);",
    )
    .await;
    assert_parse_error_with_hint(is_error, &out);
}

/// Row 5: large `String.raw` payload whose stray tokens (a payload backtick)
/// confuse the `const … = …` binding parse ("got 'value' in lexical
/// declaration binding list").
#[tokio::test]
async fn parse_hint_stray_tokens_in_binding_list() {
    let m = manager().await;
    let (is_error, out) = exec_raw(
        &m,
        "const summary = String.raw`Benchmarks:\n- 10`s faster than baseline\n- final index value: 42`;\nrecord_result(summary);",
    )
    .await;
    assert_parse_error_with_hint(is_error, &out);
}

/// Row 6: work-summary payload with a backticked path fragment
/// (`a/Users/wanjun/Desktop/starsign master`) parsed as code.
#[tokio::test]
async fn parse_hint_backticked_path_fragment_in_summary() {
    let m = manager().await;
    let (is_error, out) = exec_raw(
        &m,
        "const workSummary = String.raw`Files touched: `a/Users/wanjun/Desktop/starsign master` index rebuilt`;\nrecord_result(workSummary);",
    )
    .await;
    assert_parse_error_with_hint(is_error, &out);
}

/// Review follow-up on #23: a script that uses `String.raw` CORRECTLY but has
/// an unrelated syntax error elsewhere must NOT get the template hint — the
/// error is outside the template span, so blaming the payload would send the
/// model off rewriting the one part of the script that is fine.
#[tokio::test]
async fn parse_hint_not_added_for_unrelated_error_despite_valid_string_raw() {
    let m = manager().await;
    let (is_error, out) = exec_raw(
        &m,
        "const path = String.raw`/tmp/report.txt`;\nrecord_result( this is not valid js );",
    )
    .await;
    assert!(is_error, "the typo must still fail the parse, got: {out}");
    assert!(
        out.contains("Parse error"),
        "still a parse error, got: {out}"
    );
    assert!(
        !out.contains("String.raw does NOT make"),
        "unrelated syntax error must not get the template hint, got: {out}"
    );
}

#[tokio::test]
async fn case22_autovisualiser_blob_resource_is_collected_and_rendered_inline() {
    // A tool (autovisualiser) that returns a ui:// blob resource for the User
    // plus an Assistant-audience text label. The blob must be collected and
    // appended to the execute_code result as a Resource (for inline UI render),
    // while the JS script sees the Assistant text label as its result.
    let temp_dir = tempfile::tempdir().unwrap();
    let session_manager = Arc::new(biorouter::session::SessionManager::new(temp_dir.keep()));
    let m = Arc::new(ExtensionManager::new(
        Arc::new(Mutex::new(None)),
        session_manager,
    ));
    m.add_extension(ExtensionConfig::Builtin {
        name: "autovisualiser".to_string(),
        description: "autovis".to_string(),
        display_name: Some("Auto Visualiser".to_string()),
        timeout: Some(300),
        bundled: Some(true),
        available_tools: vec![],
    })
    .await
    .expect("add autovisualiser");
    m.add_extension(ExtensionConfig::Platform {
        name: "code_execution".to_string(),
        description: "Execute JavaScript code in a sandboxed environment".to_string(),
        bundled: Some(true),
        available_tools: vec![],
    })
    .await
    .expect("add code_execution");

    let call = CallToolRequestParams {
        task: None,
        meta: None,
        name: "code_execution__execute_code".into(),
        arguments: Some(object!({ "code": r#"
            import { render_figure } from "autovisualiser";
            const r = render_figure({ kind: "chart", data: { type: "bar", title: "T", labels: ["a","b"], datasets: [{ label: "x", data: [1,2] }] } });
            record_result(r);
        "# })),
    };
    let result = m
        .dispatch_tool_call(
            SESSION,
            call,
            biorouter::privacy::CallCapability::public_enforced(),
            CancellationToken::new(),
        )
        .await
        .expect("dispatch")
        .result
        .await
        .expect("tool result");

    assert!(!result.is_error.unwrap_or(false), "should succeed");
    // The blob resource must be appended for inline rendering.
    let has_resource = result
        .content
        .iter()
        .any(|c| matches!(&c.raw, RawContent::Resource(_)));
    assert!(has_resource, "ui:// chart resource should be appended");
    // And the script's textual result references the chart (the Assistant label),
    // not a doubled/empty value.
    let text = result
        .content
        .iter()
        .filter_map(|c| match &c.raw {
            RawContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.to_lowercase().contains("chart") || text.to_lowercase().contains("inline"),
        "result should reflect the chart label, got: {text}"
    );
}

#[tokio::test]
async fn case23_agent_drafter_preview_and_launch_metadata_survive_execute_code() {
    let temp_dir = tempfile::tempdir().unwrap();
    let session_manager = Arc::new(biorouter::session::SessionManager::new(
        temp_dir.path().join("sessions"),
    ));
    let m = Arc::new(ExtensionManager::new(
        Arc::new(Mutex::new(None)),
        session_manager,
    ));
    m.add_inprocess_server(
        "agent_drafter",
        biorouter_mcp::AgentDrafterServer::with_root(temp_dir.path().join("apps")),
    )
    .await
    .expect("add agent drafter");
    m.add_extension(ExtensionConfig::Platform {
        name: "code_execution".to_string(),
        description: "Execute JavaScript code in a sandboxed environment".to_string(),
        bundled: Some(true),
        available_tools: vec![],
    })
    .await
    .expect("add code_execution");

    let call = CallToolRequestParams {
        task: None,
        meta: None,
        name: "code_execution__execute_code".into(),
        arguments: Some(object!({ "code": r#"
            import { create_app, launch_app } from "agent_drafter";
            create_app({ title: "Nested Preview", id: "nested-preview", kind: "static" });
            const launched = launch_app({ id: "nested-preview" });
            record_result(launched);
        "# })),
    };
    let result = m
        .dispatch_tool_call(
            SESSION,
            call,
            biorouter::privacy::CallCapability::public_enforced(),
            CancellationToken::new(),
        )
        .await
        .expect("dispatch")
        .result
        .await
        .expect("tool result");

    assert!(
        !result.is_error.unwrap_or(false),
        "should succeed, got: {:?}",
        result.content
    );
    assert!(
        result
            .content
            .iter()
            .any(|content| matches!(&content.raw, RawContent::Resource(_))),
        "Agent Drafter preview resources should be appended"
    );
    assert!(
        result
            .content
            .iter()
            .any(|content| matches!(&content.raw, RawContent::ResourceLink(_))),
        "Agent Drafter launch link should be appended"
    );
    let meta = result.meta.expect("launch metadata");
    assert_eq!(
        meta.0
            .get("biorouter/app-path")
            .and_then(|value| value.as_str()),
        Some("/apps/nested-preview/")
    );
    assert_eq!(
        meta.0
            .get("biorouter/app-paths")
            .and_then(|value| value.as_array())
            .map(Vec::len),
        Some(1)
    );
}

#[tokio::test]
async fn case21_chained_data_real_dataflow() {
    let m = manager().await;
    let (_dir, dir_s) = workdir_tempdir();
    let src_s = format!("{dir_s}/src.txt");
    let dst_s = format!("{dir_s}/dst.txt");
    let dst = std::path::PathBuf::from(&dst_s);
    // The README example use case: read one file, transform, write another.
    let out = exec(
        &m,
        &format!(
            r#"
            import {{ text_editor }} from "developer";
            text_editor({{ command: "write", path: "{src_s}", file_text: "hello world\n" }});
            const content = String(text_editor({{ command: "view", path: "{src_s}" }}));
            // Extract the actual line (view output may include line numbers/framing).
            const upper = content.toUpperCase();
            text_editor({{ command: "write", path: "{dst_s}", file_text: upper }});
            record_result({{ done: true }});
            "#
        ),
    )
    .await;
    assert!(out.contains("done"), "got: {out}");
    let dst_content = std::fs::read_to_string(&dst).unwrap();
    assert!(
        dst_content.contains("HELLO WORLD"),
        "dst should contain uppercased content, got: {dst_content}"
    );
}

// ---------------------------------------------------------------------------
// Sandbox module/call errors must be self-correcting on the real dispatch path.
//
// The unit tests exercise `run_js_module` directly; these drive the same
// failures through `dispatch_tool_call`, so they assert the text the agent
// actually receives as a tool result.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn case24_unknown_module_error_names_the_real_importable_modules() {
    let m = manager().await;
    let (is_error, out) = exec_raw(
        &m,
        r#"
        import fs from "fs";
        record_result(fs.readFileSync("/etc/hosts", "utf8"));
        "#,
    )
    .await;

    assert!(is_error, "importing a non-existent module must be an error");
    assert!(
        out.contains(r#"Module "fs" could not be found"#),
        "must name the failing module, got: {out}"
    );
    // The live inventory, not a hardcoded list: this manager has developer.
    assert!(
        out.contains("Importable modules are exactly:") && out.contains("developer"),
        "must list the modules this session really has, got: {out}"
    );
    assert!(
        out.contains(r#"import { shell, text_editor } from "developer""#),
        "a Node builtin guess must be redirected to developer, got: {out}"
    );
}

/// Run `execute_code` and return the full `CallToolResult` (content + meta).
async fn exec_result(m: &Arc<ExtensionManager>, code: &str) -> rmcp::model::CallToolResult {
    let call = CallToolRequestParams {
        task: None,
        meta: None,
        name: "code_execution__execute_code".into(),
        arguments: Some(object!({ "code": code })),
    };
    m.dispatch_tool_call(
        SESSION,
        call,
        biorouter::privacy::CallCapability::public_enforced(),
        CancellationToken::new(),
    )
    .await
    .expect("dispatch")
    .result
    .await
    .expect("tool result")
}

// ---------------------------------------------------------------------------
// Issue #28 — multi-tool transparency: a failing sub-call must name its tool,
// and the result meta must carry the executed-calls telemetry for the UI.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn case26_failing_sub_call_error_names_the_tool() {
    let m = manager().await;
    let (is_error, out) = exec_raw(
        &m,
        r#"
        import { shell } from "developer";
        const r = shell({ command: "cat /nonexistent/path/xyz123" });
        record_result(r);
        "#,
    )
    .await;

    assert!(is_error, "an uncaught tool failure must error, got: {out}");
    assert!(
        out.contains("Tool error from developer__shell"),
        "the step-level error must name the failing sub-call's tool, got: {out}"
    );
}

#[tokio::test]
async fn case27_result_meta_records_executed_sub_calls() {
    let m = manager().await;
    let result = exec_result(
        &m,
        r#"
        import { shell } from "developer";
        const greeting = shell({ command: "echo hello" });
        let failure = "none";
        try {
            shell({ command: "cat /nonexistent/path/xyz123" });
        } catch (e) {
            failure = "caught";
        }
        record_result({ greeting: String(greeting).trim(), failure });
        "#,
    )
    .await;

    assert!(
        !result.is_error.unwrap_or(false),
        "the script recovered, so the step succeeds: {:?}",
        result.content
    );

    let meta = result.meta.expect("executed-call telemetry meta");
    let calls = meta
        .0
        .get("biorouter/tool-calls")
        .and_then(|value| value.as_array())
        .expect("biorouter/tool-calls array");
    assert_eq!(calls.len(), 2, "both sub-calls recorded: {calls:?}");

    let ok_call = &calls[0];
    assert_eq!(ok_call["tool"], "developer__shell");
    assert_eq!(ok_call["status"], "ok");
    assert!(
        ok_call["args"]
            .as_str()
            .unwrap_or_default()
            .contains("echo hello"),
        "exact args recorded: {ok_call:?}"
    );
    assert!(
        ok_call["result_bytes"].as_u64().unwrap_or(0) > 0,
        "success records the result size: {ok_call:?}"
    );

    let failed_call = &calls[1];
    assert_eq!(failed_call["tool"], "developer__shell");
    assert_eq!(failed_call["status"], "error");
    assert!(
        failed_call["args"]
            .as_str()
            .unwrap_or_default()
            .contains("/nonexistent/path/xyz123"),
        "exact args recorded: {failed_call:?}"
    );
    // The recorded error is the USER-audience text of the failed result —
    // shell tags a user copy of its output — never the script-facing
    // (assistant-audience) error string, and not the sanitized placeholder
    // reserved for tools that produce no user-visible text.
    let recorded_error = failed_call["error"].as_str().unwrap_or_default();
    assert!(
        recorded_error.contains("No such file"),
        "the user-audience error text is recorded: {failed_call:?}"
    );
    assert!(
        !recorded_error.contains("details hidden"),
        "a user-visible error is recorded verbatim, not sanitized: {failed_call:?}"
    );
}

#[tokio::test]
async fn case28_error_result_still_carries_call_telemetry() {
    let m = manager().await;
    let result = exec_result(
        &m,
        r#"
        import { shell } from "developer";
        shell({ command: "cat /nonexistent/path/xyz123" });
        record_result("unreachable");
        "#,
    )
    .await;

    assert!(
        result.is_error.unwrap_or(false),
        "the uncaught failure must error the step"
    );
    let meta = result
        .meta
        .expect("telemetry must survive on the error result: it matters most there");
    let calls = meta
        .0
        .get("biorouter/tool-calls")
        .and_then(|value| value.as_array())
        .expect("biorouter/tool-calls array");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["status"], "error");
    assert_eq!(calls[0]["tool"], "developer__shell");
}

#[tokio::test]
async fn case25_calling_a_non_function_explains_itself() {
    let m = manager().await;
    // Boa answers any call on a non-function with a bare "not a callable
    // function" — no value, no call site. That is the message that sent the
    // model round in circles before falling back to a Node builtin, so the
    // dispatch path must carry the explanation with it.
    let (is_error, out) = exec_raw(
        &m,
        r#"
        import { shell } from "developer";
        const result = shell({ command: "echo hi" });
        record_result(result.no_such_method());
        "#,
    )
    .await;

    assert!(
        is_error,
        "calling a non-function must be an error, got: {out}"
    );
    assert!(
        out.contains("not a callable function"),
        "the engine's own message must survive, got: {out}"
    );
    assert!(
        out.contains("JSON results are parsed objects")
            && out.contains("imported module member")
            && out.contains("One possible cause"),
        "the opaque engine error must carry its explanation, got: {out}"
    );
    assert!(
        out.contains("JSON.stringify"),
        "must name the recovery, got: {out}"
    );
}

// ---------------------------------------------------------------------------
// Issue #56 Gate C — the `execute_code` bridge, driven by a REAL script.
//
// `code_execution_extension::gate_c_bridge_tests` calls `dispatch_sub_call`
// directly with a capability it builds itself, which pins the refusal but not
// the propagation: an implementation that hard-coded a tier anywhere between
// `McpMeta` and that function would still pass it. These two run the whole
// chain — `dispatch_tool_call` → `McpMeta.capability` →
// `CodeExecutionClient::call_tool` → `handle_execute_code` → `run_tool_handler`
// → `dispatch_sub_call` → Gate C — with the outer call admitted the way the
// route admits one, so the tier the gate reads is the tier the outer dispatch
// carried and nothing in between may substitute its own.
// ---------------------------------------------------------------------------

/// `manager()` plus a private extension, admitted through the production path
/// (`add_inprocess_server` inserts into the SAME `extensions` map, so the tier
/// is stamped by the admission code rather than poked into the record).
async fn manager_with_a_private_extension() -> Arc<ExtensionManager> {
    let m = manager().await;
    m.add_inprocess_server(
        "ucsfomopagent",
        biorouter_mcp::datasql::server::DataSqlServer::new(std::collections::HashMap::new()),
    )
    .await
    .expect("inject the private extension");
    m
}

/// A provider that answers nothing and only exists to carry a tier.
///
/// `CallCapability::for_test` is `#[cfg(test)] pub(crate)`, deliberately: Task
/// 51's census counts the two production constructors under `crates/*/src/` and
/// a test spelling either of them would be indistinguishable from an entry
/// nobody classified. An integration test is outside that window but still
/// cannot see `for_test`, so the private capability below is built the way
/// production builds one — by sampling a bound provider.
struct TieredProvider(biorouter::privacy::ProviderTier);

#[async_trait::async_trait]
impl biorouter::providers::base::Provider for TieredProvider {
    fn metadata() -> biorouter::providers::base::ProviderMetadata {
        biorouter::providers::base::ProviderMetadata::new(
            "tiered",
            "Tiered",
            "",
            "tiered-model",
            vec![],
            "",
            vec![],
        )
    }

    fn get_name(&self) -> &str {
        "tiered"
    }

    fn tier(&self) -> biorouter::privacy::ProviderTier {
        self.0
    }

    /// ⚠ **A private double must state an affiliation, because every real
    /// private provider does** — issue #56 DR-26, Task 48.
    ///
    /// `Some(..)` exactly while a provider's tier is Private is a property of
    /// this build rather than an accident: both deciders route *through* the
    /// tier predicate (`ucsf_gateway_affiliation`, `self_hosted_affiliation`)
    /// and `LeadWorkerProvider` folds both halves. Leaving this on the trait
    /// default gives the one pairing DR-26's vocabulary says cannot exist —
    /// Private tier, affiliation `None` — which the gate treats as *unstated*
    /// rather than as *unconstrained*, so this double would be refused at
    /// `ucsfomopagent` for a reason these tests are not about.
    ///
    /// `Local` because it is DR-26's identity element: the one model
    /// affiliation compatible with every extension, which keeps the assertions
    /// below on the TIER axis they were written for.
    fn affiliation(&self) -> Option<biorouter::privacy::ModelAffiliation> {
        match self.0 {
            biorouter::privacy::ProviderTier::Private => {
                Some(biorouter::privacy::ModelAffiliation::Local)
            }
            biorouter::privacy::ProviderTier::Public => None,
        }
    }

    async fn complete_with_model(
        &self,
        _model_config: &biorouter::model::ModelConfig,
        _system: &str,
        _messages: &[biorouter::conversation::message::Message],
        _tools: &[rmcp::model::Tool],
    ) -> Result<
        (
            biorouter::conversation::message::Message,
            biorouter::providers::base::ProviderUsage,
        ),
        biorouter::providers::errors::ProviderError,
    > {
        unreachable!("a script's sub-call never completes a model turn")
    }

    fn get_model_config(&self) -> biorouter::model::ModelConfig {
        biorouter::model::ModelConfig::new_or_fail("tiered-model")
    }
}

/// The capability a private-model turn carries, sampled the way the agent loop
/// samples it rather than constructed.
async fn private_capability() -> biorouter::privacy::CallCapability {
    let provider: biorouter::agents::types::SharedProvider = Arc::new(Mutex::new(Some(Arc::new(
        TieredProvider(biorouter::privacy::ProviderTier::Private),
    ))));
    biorouter::privacy::CallCapability::sample(&provider).await
}

/// The refusal Gate C produces for this pair, rebuilt from the pure function.
///
/// Asserting on the extension's NAME alone would pass on a fixture that never
/// loaded it — `Tool 'ucsfomopagent__data_sources' not found` contains the name
/// too — and would go on passing after Gate C was deleted.
fn gate_c_refusal_text() -> String {
    biorouter::privacy::refusal::privacy_refusal(
        "ucsfomopagent",
        biorouter::privacy::ProviderTier::Private,
        biorouter::privacy::ProviderTier::Public,
    )
    .expect("a public caller on a private extension is refused")
    .message
    .to_string()
}

/// ⚠ **Re-baselined by Task 16 (Gate E), and the change of premise is the
/// point.**
///
/// This test used to import `ucsfomopagent`, call one of its tools, catch Gate
/// C's refusal and assert on the caught text. That whole shape depended on the
/// private module being IMPORTABLE by a public caller — i.e. on its tool names
/// and signatures being in the bridge's catalogue, which `search_modules` and
/// `read_module` serve on demand. Gate E takes the module out of that catalogue,
/// so a public caller can no longer reach the point where Gate C would speak.
/// The refusal now arrives one layer earlier and one layer stronger: the module
/// does not exist as far as this caller is concerned.
///
/// Nothing is lost by the change. The exact property this test used to hold —
/// Gate C's refusal reaching a script's sub-call intact and not laundered as a
/// user decline — is asserted directly, at the only level where a public caller
/// can still get there, by
/// `agents::code_execution_extension::gate_c_bridge_tests::the_execute_code_bridge_cannot_reach_a_private_extension`.
/// What is asserted here instead is Gate E's own guarantee, which that test
/// cannot see: no tool NAME, SIGNATURE or DESCRIPTION of the private server
/// reaches the script at all.
#[tokio::test]
async fn case29_a_script_cannot_reach_a_private_extension_from_a_public_call() {
    let m = manager_with_a_private_extension().await;
    // `exec_raw`, not `exec`: the import itself fails now, so `execute_code`
    // reports an error result rather than a caught string.
    let (is_error, out) = exec_raw(
        &m,
        r#"
        import * as omop from "ucsfomopagent";
        let seen = "the call was not refused";
        try {
            omop.data_sources({});
        } catch (e) {
            seen = String(e);
        }
        record_result(seen);
        "#,
    )
    .await;

    assert!(
        is_error,
        "a public caller must not be able to import the private module: {out}"
    );
    assert!(
        !out.contains("the call was not refused"),
        "the script must not have reached the tool at all, got: {out}"
    );
    // Gate E proper: the private server's tool names and descriptions are the
    // content being withheld, and `data_sources` is one of them. The module name
    // itself is echoed by the resolver's error and is an existence disclosure
    // only, which DR-7 puts out of scope.
    assert!(
        !out.contains("data_sources"),
        "a private server's tool names must not reach a public model, got: {out}"
    );
    assert!(
        !out.contains("The user has declined"),
        "a privacy refusal must not be laundered as a decline, got: {out}"
    );
}

/// Dispatch one of the bridge's discovery tools on a given capability and join
/// its text content. Never asserts — the two callers below disagree about what
/// the right answer is, which is the entire point.
async fn catalogue_text(
    m: &Arc<ExtensionManager>,
    tool: &str,
    args: rmcp::model::JsonObject,
    cap: biorouter::privacy::CallCapability,
) -> String {
    let dispatched = m
        .dispatch_tool_call(
            SESSION,
            CallToolRequestParams {
                task: None,
                meta: None,
                name: tool.to_string().into(),
                arguments: Some(args),
            },
            cap,
            CancellationToken::new(),
        )
        .await
        .expect("dispatch");
    let result = dispatched.result.await.expect("tool result");
    result
        .content
        .iter()
        .filter_map(|c| match &c.raw {
            RawContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Gate E on the bridge's two explicit discovery tools, which is where a model
/// that has NOT guessed a name would look. They serve tool names, signatures and
/// descriptions out of the same catalogue, so they are as much Gate E's business
/// as the system prompt is.
///
/// **Each tool is asserted in both directions, in one test.** A lone negative
/// (`!contains("data_sources")`) is satisfied by any bridge that has stopped
/// working: a tool that errors, returns nothing, or a fixture that quietly
/// stopped loading `ucsfomopagent` all make it green while asserting nothing at
/// all — and these two tools are exactly the surface this test exists to cover,
/// so `case30`'s positive control over `execute_code` does not reach them. The
/// private arm below is that control: the same tool, the same arguments, the
/// same fixture, admitted on a private capability, must NAME the tool the public
/// arm may not see.
#[tokio::test]
async fn case29b_the_module_catalogue_never_names_a_private_extension_to_a_public_call() {
    let m = manager_with_a_private_extension().await;
    let private = private_capability().await;

    for (tool, args) in [
        (
            "code_execution__search_modules",
            object!({ "terms": ["data", "sources", "sql"] }),
        ),
        (
            "code_execution__read_module",
            object!({ "module_path": "ucsfomopagent" }),
        ),
    ] {
        let public_text = catalogue_text(
            &m,
            tool,
            args.clone(),
            biorouter::privacy::CallCapability::public_enforced(),
        )
        .await;
        assert!(
            !public_text.contains("data_sources"),
            "{tool} handed a public model a private server's tool: {public_text}"
        );

        let private_text = catalogue_text(&m, tool, args, private).await;
        assert!(
            private_text.contains("data_sources"),
            "the positive control failed: {tool} does not name the private \
             server's tool even to a caller entitled to it, so the assertion \
             above proves nothing: {private_text}"
        );
    }
}

/// The other direction, so the assertion above cannot be satisfied by a bridge
/// that is simply broken for this extension: the same script, the same tool,
/// admitted on a private capability, runs.
#[tokio::test]
async fn case30_a_private_script_still_reaches_the_private_extension() {
    let m = manager_with_a_private_extension().await;
    let call = CallToolRequestParams {
        task: None,
        meta: None,
        name: "code_execution__execute_code".into(),
        arguments: Some(object!({
            "code": r#"
            import * as omop from "ucsfomopagent";
            record_result(omop.data_sources({}));
            "#
        })),
    };
    let result = m
        .dispatch_tool_call(
            SESSION,
            call,
            // The one place these tests need a capability that is NOT the
            // route's: a private caller.
            private_capability().await,
            CancellationToken::new(),
        )
        .await
        .expect("dispatch")
        .result
        .await
        .expect("tool result");

    let text = result
        .content
        .iter()
        .filter_map(|c| match &c.raw {
            RawContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !result.is_error.unwrap_or(false),
        "a private caller must reach a private extension from a script: {text}"
    );
    assert!(
        !text.contains(&gate_c_refusal_text()),
        "a private caller must not be refused: {text}"
    );
}
