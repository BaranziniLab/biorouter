//! End-to-end tests for reaching `chatrecall` **through the `code_execution` JS
//! sandbox** — the only route the model has once `code_execution` is enabled
//! (`reply_parts::survives_code_execution_filter` drops every ordinary
//! extension tool from the model's list, `chatrecall` included, and
//! `code_execution` is `default_enabled: true`).
//!
//! "Every other tool" would be wrong: that filter keeps the families a script
//! cannot express — the spawn tool, `workspace__*`, the `platform__*` tools,
//! `workflow__final_output` and the interface's frontend tools — because they
//! are absent from the importable-module catalogue and would otherwise be
//! reachable from nowhere. `chatrecall` is not one of them; it is in the
//! catalogue, which is exactly why this file exists.
//!
//! Issue #93: `chatrecall` is the one built-in whose extension key equals its
//! own tool name, so `create_server_module`'s server-name export landed on the
//! same Boa module binding as the tool export and overwrote the function with a
//! plain object. Every documented call form then threw
//! `TypeError: not a callable function`, and the tool's Rust handler was never
//! entered — a total outage for chat recall in the shipped default config.
//!
//! These tests deliberately go through `dispatch_tool_call` with a *real*
//! `ExtensionManager`, a *real* `SessionManager` and *real* seeded transcripts,
//! so they fail if any layer between the JS binding and the SQLite FTS index is
//! wrong — not just the binding.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::model::{CallToolRequestParams, RawContent};
use rmcp::object;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use biorouter::agents::extension::ExtensionConfig;
use biorouter::agents::extension_manager::ExtensionManager;
use biorouter::conversation::message::Message;
use biorouter::session::{SessionManager, SessionType};

const CALLER: &str = "chatrecall-ce-caller";

struct Harness {
    _temp: tempfile::TempDir,
    sm: Arc<SessionManager>,
    manager: Arc<ExtensionManager>,
}

impl Harness {
    /// `chatrecall` + `code_execution`, both as platform extensions, over one
    /// isolated session store. Deliberately does NOT add `developer`: this must
    /// pass with chatrecall as the only non-code_execution extension.
    async fn new() -> Self {
        let _temp = tempfile::TempDir::new().unwrap();
        let sm = Arc::new(SessionManager::new(_temp.path().to_path_buf()));
        let manager = Arc::new(ExtensionManager::new(
            Arc::new(Mutex::new(None)),
            Arc::clone(&sm),
        ));

        manager
            .add_extension(ExtensionConfig::Platform {
                name: "chatrecall".to_string(),
                description: "Search past conversations".to_string(),
                bundled: Some(true),
                available_tools: vec![],
            })
            .await
            .expect("add chatrecall");

        manager
            .add_extension(ExtensionConfig::Platform {
                name: "code_execution".to_string(),
                description: "Execute JavaScript code in a sandboxed environment".to_string(),
                bundled: Some(true),
                available_tools: vec![],
            })
            .await
            .expect("add code_execution");

        Self { _temp, sm, manager }
    }

    /// Seed one past conversation the search is expected to find.
    async fn seed(&self, name: &str, texts: &[&str]) -> String {
        let s = self
            .sm
            .create_session(
                PathBuf::from("/tmp/seeded"),
                name.to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        for t in texts {
            self.sm
                .add_message(&s.id, &Message::user().with_text(*t))
                .await
                .unwrap();
        }
        s.id
    }

    async fn corrupt_stored_content(&self, session_id: &str) {
        let options = SqliteConnectOptions::new()
            .filename(
                self._temp
                    .path()
                    .join(biorouter::session::session_manager::SESSIONS_FOLDER)
                    .join(biorouter::session::session_manager::DB_NAME),
            )
            .create_if_missing(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::query("UPDATE messages SET content_json = '{broken' WHERE session_id = ?")
            .bind(session_id)
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    /// Run `execute_code` exactly as the agent loop does and return
    /// `(is_error, joined_text)`.
    async fn exec(&self, code: &str) -> (bool, String) {
        let call = CallToolRequestParams {
            task: None,
            meta: None,
            name: "code_execution__execute_code".into(),
            arguments: Some(object!({ "code": code })),
        };
        let dispatched = self
            .manager
            .dispatch_tool_call(
                CALLER,
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

    /// Assert a script ran and did not hit the collision bug.
    async fn exec_ok(&self, code: &str) -> String {
        let (is_error, text) = self.exec(code).await;
        assert!(
            !text.contains("not a callable function"),
            "issue #93 regression: the chatrecall module export is not callable.\n{text}"
        );
        assert!(!is_error, "execute_code reported an error: {text}");
        text
    }
}

/// The exact call form `code_execution__read_module("chatrecall")` prints to the
/// model — `chatrecall["chatrecall"]({...})`. This is the form the reported
/// session used, and the one that threw.
#[tokio::test]
async fn read_modules_documented_call_form_reaches_chatrecall() {
    let h = Harness::new().await;
    h.seed(
        "SPOKE graph work",
        &["We queried the SPOKE biomedical knowledge graph for MS genes"],
    )
    .await;

    let text = h
        .exec_ok(
            r#"import * as chatrecall from "chatrecall";
record_result(chatrecall["chatrecall"]({ query: "SPOKE", limit: 25 }));"#,
        )
        .await;

    assert!(
        text.contains("SPOKE"),
        "the search ran but did not return the seeded SPOKE conversation:\n{text}"
    );
}

/// Every import form `read_module` / the system prompt tell the model to use
/// must reach the tool, not just the one form.
#[tokio::test]
async fn every_documented_import_form_reaches_chatrecall() {
    let h = Harness::new().await;
    h.seed("SPOKE graph work", &["SPOKE knowledge graph traversal"])
        .await;

    for (label, code) in [
        (
            "namespace bracket",
            r#"import * as chatrecall from "chatrecall";
record_result(chatrecall["chatrecall"]({ query: "SPOKE" }));"#,
        ),
        (
            "namespace dot",
            r#"import * as chatrecall from "chatrecall";
record_result(chatrecall.chatrecall({ query: "SPOKE" }));"#,
        ),
        (
            "named import",
            r#"import { chatrecall } from "chatrecall";
record_result(chatrecall({ query: "SPOKE" }));"#,
        ),
        (
            "server-named import",
            r#"import { chatrecall as srv } from "chatrecall";
record_result(srv.chatrecall({ query: "SPOKE" }));"#,
        ),
    ] {
        let text = h.exec_ok(code).await;
        assert!(
            text.contains("SPOKE"),
            "import form `{label}` did not return the seeded conversation:\n{text}"
        );
    }
}

/// `typeof` must be `function`. This is the single value that was wrong — the
/// server-name export made it `object` — and it is the cheapest possible guard.
#[tokio::test]
async fn the_chatrecall_export_is_a_function_not_an_object() {
    let h = Harness::new().await;
    let text = h
        .exec_ok(
            r#"import * as ns from "chatrecall";
record_result({ typeofExport: typeof ns.chatrecall, typeofBracket: typeof ns["chatrecall"] });"#,
        )
        .await;
    assert!(
        text.contains("\"typeofExport\": \"function\"")
            || text.contains("\"typeofExport\":\"function\""),
        "the module export must be callable, got:\n{text}"
    );
}

/// The tool's Rust handler must actually be entered and must actually search:
/// a query that matches nothing returns the "No results" answer rather than a
/// JS error, and a query that matches returns the session by name.
#[tokio::test]
async fn search_mode_returns_real_hits_and_real_misses() {
    let h = Harness::new().await;
    h.seed("Diabetes cohort", &["We built an OMOP diabetes cohort"])
        .await;
    h.seed("SPOKE graph work", &["SPOKE knowledge graph traversal"])
        .await;

    let hit = h
        .exec_ok(
            r#"import { chatrecall } from "chatrecall";
record_result(chatrecall({ query: "SPOKE" }));"#,
        )
        .await;
    assert!(
        hit.contains("SPOKE graph work"),
        "search did not name the matching session:\n{hit}"
    );
    assert!(
        !hit.contains("Diabetes cohort"),
        "search returned an unrelated session:\n{hit}"
    );

    let miss = h
        .exec_ok(
            r#"import { chatrecall } from "chatrecall";
record_result(chatrecall({ query: "zzzznotarealtermzzzz" }));"#,
        )
        .await;
    assert!(
        miss.contains("No results found"),
        "a miss must be a clean empty answer, not an error:\n{miss}"
    );
}

/// LOAD mode (`session_id`) is the tool's other half and travels the same
/// broken binding, so it needs its own coverage.
#[tokio::test]
async fn load_mode_returns_a_session_summary() {
    let h = Harness::new().await;
    let id = h
        .seed(
            "SPOKE graph work",
            &[
                "first message about SPOKE",
                "second message",
                "third message",
            ],
        )
        .await;

    let code = format!(
        r#"import {{ chatrecall }} from "chatrecall";
record_result(chatrecall({{ session_id: "{id}" }}));"#
    );
    let text = h.exec_ok(&code).await;

    assert!(
        text.contains("SPOKE graph work") && text.contains(&id),
        "load mode did not return the session summary:\n{text}"
    );
    assert!(
        text.contains("first message about SPOKE"),
        "load mode did not include the session's messages:\n{text}"
    );
}

/// The result must be usable *as data* inside the script — the model's whole
/// reason for being in the sandbox. A tool result that cannot be inspected is
/// only half fixed.
#[tokio::test]
async fn the_result_can_be_processed_inside_the_script() {
    let h = Harness::new().await;
    h.seed("SPOKE graph work", &["SPOKE knowledge graph traversal"])
        .await;

    let text = h
        .exec_ok(
            r#"import { chatrecall } from "chatrecall";
const r = chatrecall({ query: "SPOKE" });
const s = typeof r === "string" ? r : JSON.stringify(r);
record_result({ kind: typeof r, mentionsSpoke: s.includes("SPOKE"), length: s.length });"#,
        )
        .await;

    assert!(
        text.contains("\"mentionsSpoke\": true") || text.contains("\"mentionsSpoke\":true"),
        "the script could not inspect the chatrecall result:\n{text}"
    );
}

/// `read_module("chatrecall")` must print a call form that actually works.
/// Before the fix it printed `chatrecall["chatrecall"]({...})`, which threw —
/// the tool catalogue was actively teaching the model a broken incantation.
#[tokio::test]
async fn read_module_prints_a_call_form_that_executes() {
    let h = Harness::new().await;
    h.seed("SPOKE graph work", &["SPOKE knowledge graph traversal"])
        .await;

    let call = CallToolRequestParams {
        task: None,
        meta: None,
        name: "code_execution__read_module".into(),
        arguments: Some(object!({ "module_path": "chatrecall" })),
    };
    let dispatched = h
        .manager
        .dispatch_tool_call(
            CALLER,
            call,
            biorouter::privacy::CallCapability::public_enforced(),
            CancellationToken::new(),
        )
        .await
        .expect("dispatch read_module");
    let result = dispatched.result.await.expect("read_module result");
    let doc = result
        .content
        .iter()
        .filter_map(|c| match &c.raw {
            RawContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        doc.contains(r#"import * as chatrecall from "chatrecall""#),
        "read_module did not print the import line:\n{doc}"
    );
    assert!(
        doc.contains(r#"chatrecall["chatrecall"]"#),
        "read_module no longer prints the bracket call form; update this test:\n{doc}"
    );

    // Now run precisely what it told the model to run.
    let text = h
        .exec_ok(
            r#"import * as chatrecall from "chatrecall";
record_result(chatrecall["chatrecall"]({ query: "SPOKE" }));"#,
        )
        .await;
    assert!(
        text.contains("SPOKE"),
        "the printed form ran but found nothing:\n{text}"
    );
}

// ---------------------------------------------------------------------------
// Defects found while unblocking the tool. Each one was measured against a real
// 11,672-session store before it was fixed, and each had ZERO coverage.
// ---------------------------------------------------------------------------

impl Harness {
    /// A session whose messages are supplied as whole `Message` values, so a
    /// test can seed tool traffic and thinking blocks rather than only prose.
    async fn seed_messages(&self, name: &str, messages: Vec<Message>) -> String {
        let s = self
            .sm
            .create_session(
                PathBuf::from("/tmp/seeded"),
                name.to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        for m in &messages {
            self.sm.add_message(&s.id, m).await.unwrap();
        }
        s.id
    }
}

/// SEARCH must name the session. `sessions.description` has been a dead column
/// since the title moved to `sessions.name`; reading it alone rendered every hit
/// as `Session:  (ID: …)`. Measured on a real store: 11,672 sessions, 0 with a
/// description, 11,672 with a name.
#[tokio::test]
async fn search_results_carry_the_session_title() {
    let h = Harness::new().await;
    h.seed("OMOP diabetes cohort", &["cohort characterisation notes"])
        .await;

    let text = h
        .exec_ok(
            r#"import { chatrecall } from "chatrecall";
record_result(chatrecall({ query: "cohort" }));"#,
        )
        .await;

    assert!(
        text.contains("OMOP diabetes cohort"),
        "the hit must be named, not rendered as `Session:  (ID: …)`:\n{text}"
    );
    assert!(
        !text.contains("Session:  ("),
        "an empty session title leaked through:\n{text}"
    );
}

/// `after_date` is inclusive of its own day.
///
/// `messages.timestamp` is TEXT in `%F %T` form; sqlx encodes a `DateTime` as
/// RFC3339, whose 'T' sorts after the space, so binding one silently discarded
/// the whole boundary day. The reporter's `daily-meditation.yaml` scheduled job
/// searches "with a recent date range", so this quietly dropped the most recent
/// day's chats — the ones it most wanted.
#[tokio::test]
async fn date_filters_include_their_own_boundary_day() {
    let h = Harness::new().await;

    // ⚠ Take the boundary BEFORE seeding, not after. Deriving "today" from a
    // clock read that happens after the write makes the test fail if the two
    // straddle midnight UTC: the boundary would be the next day and the message
    // would sit just before it. Reading first means the message is always at or
    // after this instant, whichever day it lands on.
    let start_of_today = chrono::Utc::now().format("%Y-%m-%dT00:00:00Z").to_string();
    h.seed("SPOKE graph work", &["SPOKE knowledge graph traversal"])
        .await;

    // The message was written seconds ago, so an `after_date` of today 00:00:00
    // must still find it.
    let today = start_of_today;
    let code = format!(
        r#"import {{ chatrecall }} from "chatrecall";
record_result(chatrecall({{ query: "SPOKE", after_date: "{today}" }}));"#
    );
    let text = h.exec_ok(&code).await;

    assert!(
        text.contains("SPOKE graph work"),
        "an after_date of today 00:00 dropped a message written today:\n{text}"
    );

    // And the filter must still filter: tomorrow excludes it.
    let tomorrow = (chrono::Utc::now() + chrono::Duration::days(1))
        .format("%Y-%m-%dT00:00:00Z")
        .to_string();
    let code = format!(
        r#"import {{ chatrecall }} from "chatrecall";
record_result(chatrecall({{ query: "SPOKE", after_date: "{tomorrow}" }}));"#
    );
    let text = h.exec_ok(&code).await;
    assert!(
        text.contains("No results found"),
        "an after_date in the future must exclude everything:\n{text}"
    );
}

/// LOAD's first/last windows must not overlap. At 4 and 5 messages the old
/// arithmetic printed the middle ones twice, under two different numbers — the
/// only two sizes where the windows meet, which is why 3 and 6 looked fine.
#[test_case::test_case(4; "four_messages")]
#[test_case::test_case(5; "five_messages")]
#[test_case::test_case(6; "six_messages")]
#[tokio::test]
async fn load_mode_never_prints_a_message_twice(total: usize) {
    let h = Harness::new().await;
    let bodies: Vec<String> = (1..=total).map(|i| format!("unique-body-{i}")).collect();
    let id = h
        .seed_messages(
            "Overlap check",
            bodies
                .iter()
                .map(|b| Message::user().with_text(b.clone()))
                .collect(),
        )
        .await;

    let code = format!(
        r#"import {{ chatrecall }} from "chatrecall";
record_result(chatrecall({{ session_id: "{id}" }}));"#
    );
    let text = h.exec_ok(&code).await;

    for body in &bodies {
        // `unique-body-1` is a prefix of nothing else, but `unique-body-1` vs
        // `unique-body-10` would collide at total >= 10; the fixtures stop at 6.
        // ⚠ `== 1`, not `<= 1`. C3 changed TWO things: the `skip_count`
        // arithmetic AND the guard that decides whether the "Last Few Messages"
        // block is emitted at all. `<= 1` is blind to the second: invert the
        // guard so the block never renders and every body is seen ZERO times,
        // which `<= 1` happily accepts. At totals 4, 5 and 6 each body must
        // appear exactly once.
        let seen = text.matches(body.as_str()).count();
        assert_eq!(
            seen, 1,
            "`{body}` was printed {seen} times at total={total}; expected exactly once \
             (0 = a window vanished, 2 = the windows overlap):\n{text}"
        );
    }
}

/// LOAD must render tool traffic. `MessageContent::as_text()` returns `None` for
/// a tool call, a tool response and a thinking block, so a message carrying only
/// those printed its header and an empty body — 62% of messages in a real store.
#[tokio::test]
async fn load_mode_renders_messages_that_carry_no_plain_text() {
    let h = Harness::new().await;
    let tool_call = rmcp::model::CallToolRequestParams {
        task: None,
        meta: None,
        name: "developer__shell".into(),
        arguments: Some(object!({ "command": "rg SPOKE" })),
    };
    let id = h
        .seed_messages(
            "Tool-only session",
            vec![
                Message::user().with_text("find SPOKE"),
                Message::assistant().with_tool_request("call_1", Ok(tool_call)),
            ],
        )
        .await;

    let code = format!(
        r#"import {{ chatrecall }} from "chatrecall";
record_result(chatrecall({{ session_id: "{id}" }}));"#
    );
    let text = h.exec_ok(&code).await;

    assert!(
        text.contains("developer__shell") || text.contains("[Tool:"),
        "a tool-call message rendered as an empty body:\n{text}"
    );
    assert!(
        !text.contains("[no renderable content]"),
        "a tool call should render as tool traffic, not as a fallback:\n{text}"
    );
}

/// SEARCH clips each message. Unbounded bodies produced ~195k tokens for one
/// ordinary query at `limit: 50` — about 8x the inline cap — which pushes the
/// whole answer into a file the model then has to grep.
#[tokio::test]
async fn search_clips_long_messages_and_says_so() {
    let h = Harness::new().await;
    let long = format!("SPOKE {}", "x".repeat(20_000));
    h.seed("Long message", &[long.as_str()]).await;

    let text = h
        .exec_ok(
            r#"import { chatrecall } from "chatrecall";
record_result(chatrecall({ query: "SPOKE" }));"#,
        )
        .await;

    assert!(
        text.contains("truncated"),
        "a 20k-character message was rendered without a truncation marker:\n{}",
        text.chars().take(600).collect::<String>()
    );
    assert!(
        text.len() < 8_000,
        "the clipped result is still {} chars — the excerpt cap is not applied",
        text.len()
    );
}

/// When the search hits its own `limit`, the headline must not present the
/// capped count as the total. It used to say "Found 2 matching message(s)" when
/// 2 was simply the cap.
#[tokio::test]
async fn search_discloses_when_the_limit_truncated_the_answer() {
    let h = Harness::new().await;
    for i in 0..5 {
        h.seed(&format!("SPOKE session {i}"), &["SPOKE knowledge graph"])
            .await;
    }

    let text = h
        .exec_ok(
            r#"import { chatrecall } from "chatrecall";
record_result(chatrecall({ query: "SPOKE", limit: 2 }));"#,
        )
        .await;

    assert!(
        text.contains("at least"),
        "a capped search must not report its cap as the total:\n{text}"
    );
    assert!(
        text.contains("limit"),
        "a capped search must name the lever that widens it:\n{text}"
    );

    // And an uncapped search must NOT carry the disclosure. (Plain terms: the
    // FTS sanitiser quotes every whitespace-separated token as a literal prefix
    // term, so an "OR" written here would be searched for, not obeyed.)
    let uncapped = h
        .exec_ok(
            r#"import { chatrecall } from "chatrecall";
record_result(chatrecall({ query: "SPOKE", limit: 50 }));"#,
        )
        .await;
    assert!(
        !uncapped.contains("at least"),
        "an uncapped search must not claim it was truncated:\n{uncapped}"
    );
}

/// LOAD must be bounded too. Making LOAD render tool traffic (so a tool-only
/// message stops printing an empty body) also made it render the tool's
/// ARGUMENTS, and a `text_editor` write carries a whole file in those. Without a
/// cap, "show me the first few messages" can return a hundred kilobytes.
#[tokio::test]
async fn load_mode_bounds_a_message_that_carries_a_huge_tool_payload() {
    let h = Harness::new().await;
    let huge = "y".repeat(60_000);
    let tool_call = rmcp::model::CallToolRequestParams {
        task: None,
        meta: None,
        name: "developer__text_editor".into(),
        arguments: Some(object!({ "command": "write", "file_text": huge })),
    };
    let id = h
        .seed_messages(
            "Big write",
            vec![
                Message::user().with_text("write the file"),
                Message::assistant().with_tool_request("call_1", Ok(tool_call)),
            ],
        )
        .await;

    let code = format!(
        r#"import {{ chatrecall }} from "chatrecall";
record_result(chatrecall({{ session_id: "{id}" }}));"#
    );
    let text = h.exec_ok(&code).await;

    assert!(
        text.contains("truncated"),
        "a 60k-character tool payload was rendered without a truncation marker"
    );
    assert!(
        text.len() < 20_000,
        "LOAD returned {} chars for a 2-message session — the cap is not applied",
        text.len()
    );
    // Still useful: the tool is named even though its payload was clipped.
    assert!(
        text.contains("text_editor"),
        "clipping must not hide WHICH tool ran:\n{}",
        text.chars().take(500).collect::<String>()
    );
}

/// `before_date`'s half of the date fix had no coverage at all, and it is the
/// half whose behaviour CHANGED: the old RFC3339 bind compared 'T' against the
/// stored space and so swept in the whole of the boundary day, where the new
/// bind stops at the midnight the caller actually named.
#[tokio::test]
async fn before_date_stops_at_the_instant_it_names() {
    let h = Harness::new().await;
    h.seed("SPOKE graph work", &["SPOKE knowledge graph traversal"])
        .await;

    // Midnight today: the message was written since, so it is NOT before it.
    let midnight_today = chrono::Utc::now().format("%Y-%m-%dT00:00:00Z").to_string();
    let code = format!(
        r#"import {{ chatrecall }} from "chatrecall";
record_result(chatrecall({{ query: "SPOKE", before_date: "{midnight_today}" }}));"#
    );
    assert!(
        h.exec_ok(&code).await.contains("No results found"),
        "before_date must exclude messages written after the instant it names"
    );

    // End of today: the message is before it.
    let end_of_today = chrono::Utc::now().format("%Y-%m-%dT23:59:59Z").to_string();
    let code = format!(
        r#"import {{ chatrecall }} from "chatrecall";
record_result(chatrecall({{ query: "SPOKE", before_date: "{end_of_today}" }}));"#
    );
    assert!(
        h.exec_ok(&code).await.contains("SPOKE graph work"),
        "before_date at end of day must include a message written today"
    );
}

/// A reasoning model's message is `[Thinking(very long), Text(the answer)]`.
/// Clipping the JOINED parts spends the whole budget on the reasoning and
/// truncates away the reply — so LOAD would show everything except the thing it
/// was called to show.
#[tokio::test]
async fn load_mode_does_not_let_a_long_thinking_block_bury_the_answer() {
    let h = Harness::new().await;
    let id = h
        .seed_messages(
            "Reasoned answer",
            vec![
                Message::user().with_text("how many patients?"),
                Message::assistant()
                    .with_thinking("z".repeat(30_000), "sig")
                    .with_text("THE-ANSWER-IS-4182"),
            ],
        )
        .await;

    let code = format!(
        r#"import {{ chatrecall }} from "chatrecall";
record_result(chatrecall({{ session_id: "{id}" }}));"#
    );
    let text = h.exec_ok(&code).await;

    assert!(
        text.contains("THE-ANSWER-IS-4182"),
        "the answer was clipped away by the thinking block that precedes it"
    );
    assert!(
        text.contains("truncated"),
        "the thinking block should still be marked as clipped"
    );
}

/// `limit: 0` must not read as "the user never discussed this".
#[tokio::test]
async fn a_non_positive_limit_falls_back_to_the_default() {
    let h = Harness::new().await;
    h.seed("SPOKE graph work", &["SPOKE knowledge graph traversal"])
        .await;

    for limit in ["0", "-3"] {
        let code = format!(
            r#"import {{ chatrecall }} from "chatrecall";
record_result(chatrecall({{ query: "SPOKE", limit: {limit} }}));"#
        );
        let text = h.exec_ok(&code).await;
        assert!(
            text.contains("SPOKE graph work"),
            "limit={limit} produced a false negative instead of falling back:\n{text}"
        );
    }
}

/// A rendered tool call must not carry a doubled label.
#[tokio::test]
async fn a_rendered_tool_call_is_labelled_once() {
    let h = Harness::new().await;
    let tool_call = rmcp::model::CallToolRequestParams {
        task: None,
        meta: None,
        name: "developer__shell".into(),
        arguments: Some(object!({ "command": "ls" })),
    };
    let id = h
        .seed_messages(
            "Tool label",
            vec![
                Message::user().with_text("list"),
                Message::assistant().with_tool_request("c1", Ok(tool_call)),
            ],
        )
        .await;

    let code = format!(
        r#"import {{ chatrecall }} from "chatrecall";
record_result(chatrecall({{ session_id: "{id}" }}));"#
    );
    let text = h.exec_ok(&code).await;
    assert!(
        !text.contains("Tool: Tool:"),
        "the tool label is doubled:\n{text}"
    );
    assert!(
        text.contains("developer__shell"),
        "the tool must still be named:\n{text}"
    );
}

/// The truncation disclosure must survive a row that fails to render.
///
/// It used to be derived from the count of messages that came back AFTER
/// rendering, so a search that really did hit its `LIMIT` could report one fewer
/// and silently drop its own warning. Deriving it from the raw row count fixes
/// that; this pins the ordinary case so the derivation cannot quietly regress to
/// the rendered count.
#[tokio::test]
async fn the_cap_disclosure_is_derived_from_rows_not_from_rendered_messages() {
    let h = Harness::new().await;
    let corrupt = h
        .seed("SPOKE corrupt session", &["SPOKE knowledge graph"])
        .await;
    h.corrupt_stored_content(&corrupt).await;
    for i in 0..3 {
        h.seed(&format!("SPOKE session {i}"), &["SPOKE knowledge graph"])
            .await;
    }

    // Exactly at the cap: SQL returned `limit` rows, so more may exist.
    let at_cap = h
        .exec_ok(
            r#"import { chatrecall } from "chatrecall";
record_result(chatrecall({ query: "SPOKE", limit: 4 }));"#,
        )
        .await;
    assert!(
        at_cap.contains("at least") && at_cap.contains("may be"),
        "a search that returned exactly `limit` rows must disclose that more may exist:\n{at_cap}"
    );
    assert!(
        at_cap.contains("1 matching message row(s) could not be rendered"),
        "the malformed matching row was silently dropped:\n{at_cap}"
    );

    // Below the cap: complete answer, no hedge.
    let below = h
        .exec_ok(
            r#"import { chatrecall } from "chatrecall";
record_result(chatrecall({ query: "SPOKE", limit: 10 }));"#,
        )
        .await;
    assert!(
        !below.contains("at least"),
        "a complete answer must not be hedged:\n{below}"
    );
}

#[tokio::test]
async fn malformed_matching_rows_never_become_a_false_no_results_answer() {
    let h = Harness::new().await;
    let corrupt = h
        .seed("SPOKE corrupt session", &["SPOKE knowledge graph"])
        .await;
    h.corrupt_stored_content(&corrupt).await;

    let text = h
        .exec_ok(
            r#"import { chatrecall } from "chatrecall";
record_result(chatrecall({ query: "SPOKE" }));"#,
        )
        .await;

    assert!(
        !text.contains("No results found"),
        "a matching row was misreported as an empty search:\n{text}"
    );
    assert!(text.contains("Found 1 matching message row(s)"));
    assert!(text.contains("none could be rendered"));
    assert!(text.contains("malformed or unsupported"));
}

/// The excerpt must contain the term that matched.
///
/// bm25 can rank a very long message first because it discusses the query term
/// deep inside it. A head clip then shows the unrelated opening — the model is
/// told the message matched and shown text that does not contain the term.
#[tokio::test]
async fn a_search_excerpt_is_centred_on_the_match_not_the_start() {
    let h = Harness::new().await;
    // ⚠ The filler needs whitespace around the term. FTS5 tokenises on
    // non-alphanumerics, so gluing the needle to 20k filler characters makes it
    // part of one enormous token and nothing matches — a fixture bug that reads
    // exactly like a broken excerpt.
    let filler = vec!["alpha"; 4_000].join(" ");
    let body = format!("{filler} ribosomebiogenesis {filler}");
    h.seed("Long analysis", &[body.as_str()]).await;

    let text = h
        .exec_ok(
            r#"import { chatrecall } from "chatrecall";
record_result(chatrecall({ query: "ribosomebiogenesis" }));"#,
        )
        .await;

    assert!(
        text.contains("ribosomebiogenesis"),
        "the excerpt does not contain the term that matched:\n{}",
        text.chars().take(400).collect::<String>()
    );
    assert!(
        text.contains("earlier text not shown"),
        "a centred window must say that it skipped the head:\n{}",
        text.chars().take(400).collect::<String>()
    );
    assert!(
        text.len() < 8_000,
        "excerpt is unbounded: {} chars",
        text.len()
    );
}

/// LOAD must show a tool response's payload, not the FTS index's placeholder.
/// Half the messages in an agentic session are tool responses; "the model ran a
/// command and then something happened" is not a transcript.
#[tokio::test]
async fn load_mode_renders_a_tool_responses_payload() {
    let h = Harness::new().await;
    let tool_call = rmcp::model::CallToolRequestParams {
        task: None,
        meta: None,
        name: "developer__shell".into(),
        arguments: Some(object!({ "command": "ls" })),
    };
    let response = rmcp::model::CallToolResult::success(vec![rmcp::model::Content::text(
        "PAYLOAD-FROM-THE-TOOL",
    )]);
    let id = h
        .seed_messages(
            "Tool response",
            vec![
                Message::user().with_text("list"),
                Message::assistant().with_tool_request("c1", Ok(tool_call)),
                Message::user().with_tool_response("c1", Ok(response)),
            ],
        )
        .await;

    let code = format!(
        r#"import {{ chatrecall }} from "chatrecall";
record_result(chatrecall({{ session_id: "{id}" }}));"#
    );
    let text = h.exec_ok(&code).await;

    assert!(
        text.contains("PAYLOAD-FROM-THE-TOOL"),
        "the tool response rendered as a placeholder instead of its payload:\n{text}"
    );
}

/// Centring must not panic on a non-ASCII transcript.
///
/// The match offset comes from a lowercased copy, and `to_lowercase` is not
/// length-preserving ("İ" becomes two chars), so a byte index taken against the
/// copy points somewhere else in the original — potentially mid-codepoint, which
/// is a panic, not a wrong answer.
#[tokio::test]
async fn a_centred_excerpt_survives_a_non_ascii_message() {
    let h = Harness::new().await;
    let filler = vec!["İstanbul café — 日本語のテキスト"; 900].join(" ");
    let body = format!("{filler} mitochondria {filler}");
    h.seed("Unicode analysis", &[body.as_str()]).await;

    let text = h
        .exec_ok(
            r#"import { chatrecall } from "chatrecall";
record_result(chatrecall({ query: "mitochondria" }));"#,
        )
        .await;

    assert!(
        text.contains("mitochondria"),
        "the excerpt lost the match on a non-ASCII message"
    );
}
