//! Biorouter Copilot driven through its own tools against a real fixture app (AD-21, AD-22 in
//! docs/design/copilot-agent-desktop.md).
//!
//! Ignored by default: it needs a desktop session, the engine payload and a fixture app. CI's
//! GUI jobs run it (`.github/workflows/computer-use-native.yml`):
//!
//! ```text
//! BIOROUTER_COMPUTER_USE_DIR=target/computer-use/<target>
//! BIOROUTER_COPILOT_FIXTURE='["python3","scripts/copilot-fixture-gtk.py"]'   # Linux, under Xvfb
//! BIOROUTER_COPILOT_FIXTURE='["target/copilot-fixture/BiorouterCopilotFixture.exe"]'  # Windows
//! cargo test -p biorouter-mcp --test computer_use_fixture -- --ignored --nocapture
//! ```
//!
//! Every call goes through `ComputerControllerServer`, the same MCP server the desktop app runs,
//! with the meta the host's consent gate attaches, so the wrapper's references, refusals and
//! input table are what is tested, not the engine on its own. Every outcome is read from the
//! state file the fixture app writes (scripts/copilot-fixture-gtk.py,
//! scripts/copilot-fixture-winforms.cs), never from a tool's own report. The host's consent
//! itself (the card, the grant, Stop) is covered by `security::computer_use`'s tests.
#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use biorouter_mcp::ComputerControllerServer;
use rmcp::{
    model::{CallToolRequest, CallToolRequestParams, ClientRequest, Meta, ServerResult},
    service::RunningService,
    RoleClient, ServiceExt,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const SESSION: &str = "copilot-fixture-chat";
const DRAG_TEXT: &str = "Dropped by Copilot";

struct Fixture {
    child: Child,
    work: tempfile::TempDir,
}

impl Fixture {
    fn start() -> Self {
        let argv: Vec<String> = serde_json::from_str(
            &std::env::var("BIOROUTER_COPILOT_FIXTURE")
                .expect("set BIOROUTER_COPILOT_FIXTURE to the fixture's command as a JSON array"),
        )
        .expect("BIOROUTER_COPILOT_FIXTURE is a JSON array of strings");
        let work = tempfile::Builder::new()
            .prefix("copilot-fixture-")
            .tempdir()
            .unwrap();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let child = Command::new(&argv[0])
            .args(&argv[1..])
            .arg(work.path())
            .current_dir(root)
            .stdin(Stdio::null())
            .spawn()
            .expect("the fixture app starts");
        let fixture = Self { child, work };
        let deadline = Instant::now() + Duration::from_secs(60);
        while !fixture.work.path().join("ready").exists() {
            assert!(Instant::now() < deadline, "the fixture app never showed");
            std::thread::sleep(Duration::from_millis(100));
        }
        fixture
    }

    fn state(&self) -> Value {
        for _ in 0..50 {
            if let Ok(text) = std::fs::read_to_string(self.work.path().join("state.json")) {
                if let Ok(value) = serde_json::from_str(text.trim_start_matches('\u{feff}')) {
                    return value;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the fixture's state file stayed unreadable");
    }

    /// Wait for the app itself to report `expected` under `key`.
    async fn expect(&self, key: &str, expected: Value) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state = self.state();
            if state[key] == expected {
                return state;
            }
            assert!(
                Instant::now() < deadline,
                "the fixture never reported {key} = {expected}; it reports {state}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Copilot {
    client: RunningService<RoleClient, ()>,
}

/// What a call answered: its text, and whether it was refused (a tool error or a protocol error,
/// whose message is the text).
struct Answer {
    text: String,
    refused: bool,
}

impl Copilot {
    async fn start() -> Self {
        let (client_io, server_io) = tokio::io::duplex(1 << 20);
        tokio::spawn(async move {
            if let Ok(server) = ComputerControllerServer::new().serve(server_io).await {
                let _ = server.waiting().await;
            }
        });
        Self {
            client: ().serve(client_io).await.unwrap(),
        }
    }

    async fn call_as(&self, generation: &str, grant: &str, tool: &str, arguments: Value) -> Answer {
        let mut extensions = rmcp::model::Extensions::default();
        let meta = json!({
            "biorouter-session-id": SESSION,
            "computer_use_generation": generation,
            "computer_use_grant": grant,
        });
        extensions.insert(Meta(meta.as_object().unwrap().clone()));
        let request = CallToolRequest {
            method: Default::default(),
            params: CallToolRequestParams {
                meta: None,
                name: tool.to_owned().into(),
                arguments: Some(arguments.as_object().cloned().unwrap_or_default()),
                task: None,
            },
            extensions,
        };
        let answer = tokio::time::timeout(
            Duration::from_secs(90),
            self.client
                .send_request(ClientRequest::CallToolRequest(request)),
        )
        .await
        .unwrap_or_else(|_| panic!("{tool} did not answer within 90 s"));
        let answer = match answer {
            Ok(ServerResult::CallToolResult(result)) => Answer {
                text: result
                    .content
                    .iter()
                    .filter_map(|content| content.as_text().map(|text| text.text.clone()))
                    .collect::<Vec<_>>()
                    .join("\n"),
                refused: result.is_error == Some(true),
            },
            Ok(other) => panic!("{tool} answered {other:?}"),
            Err(error) => Answer {
                text: error.to_string(),
                refused: true,
            },
        };
        println!(
            "{tool} {} -> {}{}",
            arguments,
            if answer.refused { "REFUSED " } else { "" },
            answer.text.chars().take(600).collect::<String>()
        );
        answer
    }

    async fn call(&self, generation: &str, tool: &str, arguments: Value) -> Answer {
        self.call_as(generation, "act", tool, arguments).await
    }

    async fn ok(&self, generation: &str, tool: &str, arguments: Value) -> String {
        let answer = self.call(generation, tool, arguments).await;
        assert!(!answer.refused, "{tool} was refused: {}", answer.text);
        answer.text
    }
}

/// `@s…:eN [role] "name"` lines of an observation.
fn references(observation: &str) -> Vec<(String, String, String)> {
    let line = regex::Regex::new(r#"(@s[0-9A-Za-z]+:e\d+) \[([^\]]+)\](?: "([^"]*)")?"#).unwrap();
    line.captures_iter(observation)
        .map(|found| {
            (
                found[1].to_owned(),
                found[2].to_owned(),
                found.get(3).map_or("", |name| name.as_str()).to_owned(),
            )
        })
        .collect()
}

fn reference(observation: &str, name: &str) -> String {
    references(observation)
        .into_iter()
        .find(|(_, _, found)| found == name)
        .map(|(reference, _, _)| reference)
        .unwrap_or_else(|| panic!("no reference named {name:?} in:\n{observation}"))
}

/// The fixture's password field: by its name where the platform reports one. Windows UI
/// Automation reports a password edit without its name, so there it is the one element the
/// observation marks as a password field (the same mark the refusal is decided on).
fn password_reference(observation: &str) -> String {
    if let Some((reference, _, _)) = references(observation)
        .into_iter()
        .find(|(_, _, name)| name == "FixtureSecret")
    {
        return reference;
    }
    let marked =
        regex::Regex::new(r#"(@s[0-9A-Za-z]+:e\d+) \[[^\]]+\][^\n]*\(password field\)"#).unwrap();
    let found: Vec<String> = marked
        .captures_iter(observation)
        .map(|found| found[1].to_owned())
        .collect();
    assert_eq!(
        found.len(),
        1,
        "expected exactly one password field in:\n{observation}"
    );
    found[0].clone()
}

fn scroll_reference(observation: &str) -> String {
    references(observation)
        .into_iter()
        .find(|(_, role, name)| name == "FixtureScroll" || role == "scrollarea")
        .map(|(reference, _, _)| reference)
        .unwrap_or_else(|| panic!("no scrollable element in:\n{observation}"))
}

/// The app's name as list_apps shows it: the line naming the fixture's process. Compared without
/// case, spaces or an `.exe`, because a platform may list `BiorouterCopilotFixture.exe` or
/// `Biorouter Copilot Fixture` for the process `BiorouterCopilotFixture`.
fn app_name(listing: &str, process: &str) -> String {
    let plain = |text: &str| {
        let lowered = text.to_ascii_lowercase().replace(' ', "");
        lowered
            .strip_suffix(".exe")
            .map(str::to_owned)
            .unwrap_or(lowered)
    };
    let wanted = plain(process);
    listing
        .lines()
        .map(str::trim)
        .map(|line| line.split(" (").next().unwrap_or(line))
        .find(|name| {
            let name = plain(name);
            !name.is_empty() && (name == wanted || name.starts_with(&wanted))
        })
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("list_apps does not show {process:?}:\n{listing}"))
}

fn drop_window(listing: &str) -> String {
    let line = regex::Regex::new(r#"window_id (\S+): .*"Biorouter Copilot Fixture Drop""#).unwrap();
    line.captures(listing)
        .map(|found| found[1].to_owned())
        .unwrap_or_else(|| panic!("no drop window in:\n{listing}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a desktop session, the engine payload and BIOROUTER_COPILOT_FIXTURE"]
async fn copilot_drives_the_fixture_app_through_its_own_tools() {
    let fixture = Fixture::start();
    let start = fixture.state();
    let process = start["app"]
        .as_str()
        .expect("the fixture names its process");
    let copilot = Copilot::start().await;
    let generation = "fixture-1";

    // Observe: the app is listed and its window reads as references.
    let listing = copilot.ok(generation, "list_apps", json!({})).await;
    let app = app_name(&listing, process);
    let window = copilot
        .ok(
            generation,
            "get_app_state",
            json!({"app": app, "max_depth": 30}),
        )
        .await;
    for name in [
        "FixtureInput",
        "Apply fixture",
        "Fixture option",
        "Alpha",
        "Beta",
        "Gamma",
        "Drag me",
    ] {
        reference(&window, name);
    }
    password_reference(&window);
    println!("PASS observe: list_apps and get_app_state name every fixture target");

    // Look only (AD-10): a write under an observe grant is refused and nothing reaches the app.
    let input = reference(&window, "FixtureInput");
    let looked = copilot
        .call_as(
            generation,
            "observe",
            "set_value",
            json!({"ref": input, "value": "must not land"}),
        )
        .await;
    assert!(
        looked.refused && looked.text.contains("look only"),
        "{}",
        looked.text
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(fixture.state()["input"], "");
    println!("PASS look only: a write under an observe grant is refused before the app");

    // Set a value, then click a button that copies it: both outcomes come from the app.
    copilot
        .ok(
            generation,
            "set_value",
            json!({"ref": input, "value": "Typed by Copilot"}),
        )
        .await;
    fixture.expect("input", json!("Typed by Copilot")).await;
    let window = copilot
        .ok(
            generation,
            "get_app_state",
            json!({"app": app, "max_depth": 30}),
        )
        .await;
    copilot
        .ok(
            generation,
            "click",
            json!({"ref": reference(&window, "Apply fixture")}),
        )
        .await;
    fixture.expect("applied", json!("Typed by Copilot")).await;
    copilot
        .ok(
            generation,
            "perform_secondary_action",
            json!({"ref": reference(&window, "Fixture option"), "action": "check"}),
        )
        .await;
    fixture.expect("checked", json!(true)).await;
    println!("PASS act: set_value, click and check land in the app");

    // A password field is refused (AD-12), and nothing is typed into it.
    let typed = copilot
        .call(
            generation,
            "type_text",
            json!({"ref": password_reference(&window), "text": "hunter2"}),
        )
        .await;
    assert!(
        typed.refused && typed.text.contains("password field"),
        "{}",
        typed.text
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(fixture.state()["secret_length"], 0);
    println!("PASS refusal: nothing is typed into a password field");

    // A reference from another chat is refused before anything runs (AD-09).
    let foreign = copilot
        .call_as(generation, "act", "click", json!({"ref": "@s0foreign:e1"}))
        .await;
    assert!(foreign.refused, "{}", foreign.text);

    // Drag within the app: Gamma dropped on Alpha moves to the top of the list.
    let window = copilot
        .ok(
            generation,
            "get_app_state",
            json!({"app": app, "max_depth": 30}),
        )
        .await;
    copilot
        .ok(
            generation,
            "drag",
            json!({"from_ref": reference(&window, "Gamma"), "to_ref": reference(&window, "Alpha")}),
        )
        .await;
    fixture
        .expect("order", json!(["Gamma", "Alpha", "Beta"]))
        .await;
    let window = copilot
        .ok(
            generation,
            "get_app_state",
            json!({"app": app, "max_depth": 30}),
        )
        .await;
    copilot
        .ok(
            generation,
            "drag",
            json!({"from_ref": reference(&window, "Beta"), "to_ref": reference(&window, "Gamma")}),
        )
        .await;
    fixture
        .expect("order", json!(["Beta", "Gamma", "Alpha"]))
        .await;
    println!("PASS drag: two drops reorder the list exactly");

    // Drag between two windows: text dropped from a button into the other window's field.
    let windows = copilot
        .ok(generation, "screen_capture", json!({"list_only": true}))
        .await;
    let drop_id = drop_window(&windows);
    let drop_side = copilot
        .ok(
            generation,
            "get_app_state",
            json!({"app": app, "window_id": drop_id}),
        )
        .await;
    let target = reference(&drop_side, "FixtureDrop");
    let main = copilot
        .ok(
            generation,
            "get_app_state",
            json!({"app": app, "max_depth": 30}),
        )
        .await;
    copilot
        .ok(
            generation,
            "drag",
            json!({"from_ref": reference(&main, "Drag me"), "to_ref": target}),
        )
        .await;
    fixture.expect("dropped", json!(DRAG_TEXT)).await;
    println!("PASS drag between windows: the dragged text lands in the other window");

    // Scroll a long text by reference. A snapshot that could not prove its references unique
    // in time is refused as ambiguous, and the advice is to observe again: do that, twice at most.
    let mut attempt = 0;
    loop {
        let window = copilot
            .ok(
                generation,
                "get_app_state",
                json!({"app": app, "max_depth": 30}),
            )
            .await;
        let scrolled = copilot
            .call(
                generation,
                "scroll",
                json!({"ref": scroll_reference(&window), "direction": "down", "amount": 10}),
            )
            .await;
        if !scrolled.refused {
            break;
        }
        println!("scroll refused on attempt {attempt}; the whole observation was:\n{window}");
        attempt += 1;
        assert!(
            scrolled.text.contains("AMBIGUOUS_TARGET") && attempt < 3,
            "scroll was refused: {}",
            scrolled.text
        );
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while fixture.state()["scroll"].as_f64().unwrap_or(0.0) <= 0.0 {
        assert!(Instant::now() < deadline, "scrolling never moved the text");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    println!("PASS scroll: the long text moved");

    // Stop during a drag (AD-15): the host's Stop ends the command, the release guard lifts the
    // button, and nothing stays pressed.
    let window = copilot
        .ok(
            generation,
            "get_app_state",
            json!({"app": app, "max_depth": 30}),
        )
        .await;
    let (from, to) = (reference(&window, "Alpha"), reference(&window, "Beta"));
    let dragging = copilot.call(generation, "drag", json!({"from_ref": from, "to_ref": to}));
    let stop = async {
        tokio::time::sleep(Duration::from_millis(250)).await;
        biorouter_mcp::computer_use::stop_session(SESSION, generation).await;
    };
    let (stopped, ()) = tokio::join!(dragging, stop);
    println!("drag under Stop answered: refused={}", stopped.refused);
    fixture.expect("pointer_pressed", json!(false)).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        fixture.state()["pointer_pressed"],
        false,
        "a press outlived Stop"
    );
    // Stop ended the task: a reference observed before it is refused, never acted on.
    let after = copilot
        .call(
            generation,
            "click",
            json!({"ref": reference(&window, "Apply fixture")}),
        )
        .await;
    assert!(
        after.refused,
        "a reference from before Stop ran: {}",
        after.text
    );
    println!("PASS stop: no button stays pressed and the stopped task's references are refused");

    // A new task after Stop starts clean, and a helper never showed a console window.
    copilot.ok("fixture-2", "list_apps", json!({})).await;
    let end = fixture.state();
    assert_eq!(end["pointer_pressed"], false);
    if let Some(consoles) = end.get("console_windows") {
        assert_eq!(
            consoles,
            &json!(0),
            "a console window appeared during the run"
        );
    }
    println!("PASS: {end}");
}

#[test]
fn reference_lines_parse_as_the_wrapper_renders_them() {
    let observation = "<desktop-app app=\"x\" untrusted=\"true\">\n@s1abc:e1 [textfield] \"FixtureInput\" value=\"\" (focused)\n  @s1abc:e7 [cell] \"Alpha\"\n@s1abc:e11 [scrollarea]\n</desktop-app>";
    assert_eq!(reference(observation, "Alpha"), "@s1abc:e7");
    assert_eq!(scroll_reference(observation), "@s1abc:e11");
    assert_eq!(
        app_name("Finder (com.apple.finder)\npython3.12\n", "python3.12"),
        "python3.12"
    );
    assert_eq!(
        app_name(
            "Explorer\nBiorouter Copilot Fixture (org.biorouter.copilot-fixture)\n",
            "BiorouterCopilotFixture"
        ),
        "Biorouter Copilot Fixture"
    );
    assert_eq!(
        app_name("BiorouterCopilotFixture.exe\n", "BiorouterCopilotFixture"),
        "BiorouterCopilotFixture.exe"
    );
    assert_eq!(
        drop_window("window_id w-10: python3.12 \"Biorouter Copilot Fixture Drop\"\n"),
        "w-10"
    );
}
