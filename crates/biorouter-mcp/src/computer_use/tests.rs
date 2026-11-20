//! The wrapper end to end against a stand-in engine (a shell script that answers each command
//! from a fixture file and records its command line), with stubbed host probes. Nothing here
//! touches the real desktop.
use super::*;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicBool, Ordering};

const ENGINE: &str = r#"#!/bin/sh
dir=$(cd "$(dirname "$0")" && pwd)
printf '%s\n' "$*" >> "$dir/calls.log"
printf '%s\n' "$AGENT_DESKTOP_HOME" > "$dir/home"
cmd=$1
for last; do :; done
if [ "$cmd" = screenshot ]; then cp "$dir/shot.png" "$last"; fi
if [ -f "$dir/block-$cmd" ]; then
  trap 'echo released > "$dir/released"; exit 0' TERM
  echo $$ > "$dir/blocked.pid"
  while :; do sleep 1; done
fi
if [ -f "$dir/$cmd.json" ]; then cat "$dir/$cmd.json"; else
  echo '{"version":"2.6","ok":true,"command":"'"$cmd"'","data":{"action":"'"$cmd"'","disposition":{"delivery":"delivered_verified","retry":"unsafe"}}}'
fi
"#;

struct Stub {
    owned: Vec<u32>,
    locked: AtomicBool,
    secure: AtomicBool,
}

impl HostProbe for Stub {
    fn identity(&self) -> Identity {
        Identity::from_parts(self.owned.clone(), None, HashMap::new())
    }
    fn screen_locked(&self) -> bool {
        self.locked.load(Ordering::SeqCst)
    }
    fn secure_input(&self) -> bool {
        self.secure.load(Ordering::SeqCst)
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    probe: Arc<Stub>,
    runtime: SessionRuntime,
}

fn envelope(command: &str, data: Value) -> String {
    json!({"version":"2.6","ok":true,"command":command,"data":data}).to_string()
}

fn window(id: &str, pid: u32, app: &str, title: &str, bounds: (f64, f64, f64, f64)) -> Value {
    json!({"id":id,"pid":pid,"app_name":app,"title":title,"visible":true,"minimized":false,
           "bounds":{"x":bounds.0,"y":bounds.1,"width":bounds.2,"height":bounds.3}})
}

impl Fixture {
    fn new(platform: Platform) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let engine = dir.path().join("engine");
        std::fs::create_dir_all(&engine).unwrap();
        let executable = engine.join("agent-desktop");
        std::fs::write(&executable, ENGINE).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2000, 1000)
            .write_to(&mut png, image::ImageOutputFormat::Png)
            .unwrap();
        std::fs::write(engine.join("shot.png"), png.into_inner()).unwrap();
        let fixture_files = [
            (
                "list-apps",
                envelope(
                    "list-apps",
                    json!({"apps":[
                        {"name":"TextEdit","bundle_id":"com.apple.TextEdit","pid":100},
                        {"name":"Biorouter","bundle_id":"com.electron.biorouter","pid":200},
                        {"name":"Terminal","bundle_id":"com.apple.Terminal","pid":300},
                        {"name":"System Settings","bundle_id":"com.apple.systempreferences","pid":400},
                        {"name":"SecurityAgent","bundle_id":"com.apple.SecurityAgent","pid":500},
                        {"name":"Finder","bundle_id":"com.apple.finder","pid":600}
                    ]}),
                ),
            ),
            (
                "list-windows",
                envelope(
                    "list-windows",
                    json!([
                        window(
                            "w1",
                            100,
                            "TextEdit",
                            "Untitled",
                            (100.0, 50.0, 1000.0, 500.0)
                        ),
                        window("wb", 200, "Biorouter", "Chat", (0.0, 0.0, 900.0, 700.0)),
                        window("wt", 300, "Terminal", "zsh", (10.0, 10.0, 500.0, 300.0)),
                        window(
                            "ws",
                            400,
                            "System Settings",
                            "Privacy & Security",
                            (20.0, 20.0, 600.0, 400.0)
                        ),
                        window("wf", 600, "Finder", "Desktop", (2000.0, 0.0, 500.0, 400.0))
                    ]),
                ),
            ),
            (
                "list-displays",
                envelope(
                    "list-displays",
                    json!([{"bounds":{"x":0.0,"y":0.0,"width":1440.0,"height":900.0},"id":"main","is_primary":true,"scale":2.0},
                                                   {"bounds":{"x":1440.0,"y":0.0,"width":1920.0,"height":1080.0},"id":"side","is_primary":false,"scale":1.0}]),
                ),
            ),
            (
                "snapshot",
                envelope(
                    "snapshot",
                    json!({"app":"TextEdit","complete":true,"snapshot_id":"s1",
                    "window":{"id":"w1","title":"Untitled"},
                    "tree":{"role":"window","name":"Untitled","children":[
                        {"role":"button","name":"Save","ref_id":"@s1:e1","available_actions":["Click"]},
                        {"role":"textfield","name":"Password","ref_id":"@s1:e2","states":["secure"]},
                        {"role":"textfield","name":"Body","ref_id":"@s1:e3","value":"hello"}
                    ]}}),
                ),
            ),
            (
                "screenshot",
                envelope(
                    "screenshot",
                    json!({"format":"png","width":2000,"height":1000,"scale_factor":2.0}),
                ),
            ),
        ];
        for (command, body) in fixture_files {
            std::fs::write(engine.join(format!("{command}.json")), body).unwrap();
        }
        let probe = Arc::new(Stub {
            owned: vec![200],
            locked: AtomicBool::new(false),
            secure: AtomicBool::new(false),
        });
        let runtime =
            SessionRuntime::for_tests(probe.clone(), platform, executable, dir.path().join("runs"));
        Self {
            dir,
            probe,
            runtime,
        }
    }

    fn engine(&self) -> PathBuf {
        self.dir.path().join("engine")
    }

    fn set(&self, command: &str, body: &str) {
        std::fs::write(self.engine().join(format!("{command}.json")), body).unwrap();
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.engine().join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn clear_calls(&self) {
        let _ = std::fs::remove_file(self.engine().join("calls.log"));
    }

    async fn call(
        &mut self,
        tool: &str,
        arguments: Value,
        grant: Grant,
    ) -> Result<CallToolResult, ErrorData> {
        self.call_in("chat", "task-1", tool, arguments, grant).await
    }

    async fn call_in(
        &mut self,
        chat: &str,
        generation: &str,
        tool: &str,
        arguments: Value,
        grant: Grant,
    ) -> Result<CallToolResult, ErrorData> {
        self.runtime
            .call(
                chat,
                generation,
                tool,
                arguments,
                grant,
                CancellationToken::new(),
            )
            .await
    }

    async fn observe(&mut self, app: &str) {
        let result = self
            .call("get_app_state", json!({"app":app}), Grant::Observe)
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
    }
}

fn text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|content| content.as_text().map(|text| text.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn refused(result: Result<CallToolResult, ErrorData>) -> String {
    match result {
        Err(error) => error.message.to_string(),
        Ok(result) => panic!("expected a refusal, got {}", text(&result)),
    }
}

#[tokio::test]
async fn look_only_refuses_every_act_call_before_anything_runs() {
    let mut fx = Fixture::new(Platform::MacOs);
    for (tool, arguments) in [
        ("click", json!({"ref":"@s1:e1"})),
        ("type_text", json!({"ref":"@s1:e3","text":"x"})),
        ("drag", json!({"from_ref":"@s1:e1","to_ref":"@s1:e3"})),
        ("open_app", json!({"app":"TextEdit"})),
        ("press_key", json!({"app":"TextEdit","key":"return"})),
        (
            "perform_secondary_action",
            json!({"ref":"@s1:e1","action":"toggle"}),
        ),
    ] {
        assert_eq!(
            refused(fx.call(tool, arguments, Grant::Observe).await),
            LOOK_ONLY_REFUSAL,
            "{tool}"
        );
    }
    assert!(
        fx.calls().is_empty(),
        "nothing may run under a refused grant"
    );
    fx.observe("TextEdit").await;
    let expanded = fx
        .call(
            "perform_secondary_action",
            json!({"ref":"@s1:e1","action":"expand"}),
            Grant::Observe,
        )
        .await
        .unwrap();
    assert!(text(&expanded).starts_with("Performed expand."));
}

#[tokio::test]
async fn a_locked_screen_refuses_even_observing() {
    let mut fx = Fixture::new(Platform::MacOs);
    fx.probe.locked.store(true, Ordering::SeqCst);
    assert_eq!(
        refused(fx.call("list_apps", json!({}), Grant::Act).await),
        guard::LOCKED_REFUSAL
    );
    assert!(fx.calls().is_empty());
}

#[tokio::test]
async fn biorouter_and_security_prompts_are_never_observed_or_listed() {
    let mut fx = Fixture::new(Platform::MacOs);
    let apps = text(
        &fx.call("list_apps", json!({}), Grant::Observe)
            .await
            .unwrap(),
    );
    assert!(apps.contains("TextEdit (com.apple.TextEdit)"));
    assert!(!apps.contains("Biorouter ("), "{apps}");
    assert!(!apps.contains("SecurityAgent"));
    assert!(apps.contains("Biorouter itself and system security prompts are not listed"));
    assert!(apps.starts_with("<desktop-app app=\"running apps\" untrusted=\"true\">"));
    for app in ["Biorouter", "biorouter", "com.electron.biorouter"] {
        assert_eq!(
            refused(
                fx.call("get_app_state", json!({"app":app}), Grant::Observe)
                    .await
            ),
            guard::BIOROUTER_REFUSAL,
            "{app}"
        );
    }
    assert_eq!(
        refused(
            fx.call(
                "get_app_state",
                json!({"app":"SecurityAgent"}),
                Grant::Observe
            )
            .await
        ),
        guard::SECURITY_PROMPT_REFUSAL
    );
    assert_eq!(
        refused(
            fx.call("screen_capture", json!({"window_id":"wb"}), Grant::Observe)
                .await
        ),
        guard::BIOROUTER_REFUSAL
    );
    assert_eq!(
        refused(
            fx.call("open_app", json!({"app":"Biorouter"}), Grant::Act)
                .await
        ),
        guard::BIOROUTER_REFUSAL
    );
    assert!(!fx
        .calls()
        .iter()
        .any(|call| call.starts_with("snapshot") || call.starts_with("screenshot")));
    let windows = text(
        &fx.call("screen_capture", json!({"list_only":true}), Grant::Observe)
            .await
            .unwrap(),
    );
    assert!(windows.contains("window_id w1"));
    assert!(!windows.contains("window_id wb"), "{windows}");
    assert!(windows.contains("display 0: 1440 by 900 points (main)"));
    let missing = refused(
        fx.call(
            "get_app_state",
            json!({"app":"Nonexistent"}),
            Grant::Observe,
        )
        .await,
    );
    assert!(missing.contains("computer_use_app_not_found"));
}

#[tokio::test]
async fn references_must_come_from_this_chats_current_task() {
    let mut fx = Fixture::new(Platform::MacOs);
    let never = refused(fx.call("click", json!({"ref":"@s1:e1"}), Grant::Act).await);
    assert!(never.contains("computer_use_stale_reference"), "{never}");
    fx.observe("TextEdit").await;
    let snapshot_call = fx
        .calls()
        .into_iter()
        .find(|call| call.starts_with("snapshot"))
        .unwrap();
    assert!(snapshot_call.contains("--app=TextEdit"));
    assert!(snapshot_call.contains("--session=task-1"));
    assert!(snapshot_call.contains("--interactive-only"));
    fx.clear_calls();
    let clicked = fx
        .call("click", json!({"ref":"@s1:e1"}), Grant::Act)
        .await
        .unwrap();
    assert_eq!(text(&clicked), "Clicked. Delivered and verified.");
    assert_eq!(fx.calls(), ["click --session=task-1 -- @s1:e1"]);
    // Another chat on the same connection is refused outright.
    let other = refused(
        fx.call_in(
            "other-chat",
            "task-1",
            "click",
            json!({"ref":"@s1:e1"}),
            Grant::Act,
        )
        .await,
    );
    assert!(other.contains("session_mismatch"));
    // A new grant generation starts a new task: the old references are gone.
    let next = refused(
        fx.call_in(
            "chat",
            "task-2",
            "click",
            json!({"ref":"@s1:e1"}),
            Grant::Act,
        )
        .await,
    );
    assert!(next.contains("computer_use_stale_reference"));
}

#[tokio::test]
async fn password_fields_are_never_typed_into_set_or_cleared() {
    let mut fx = Fixture::new(Platform::MacOs);
    fx.observe("TextEdit").await;
    fx.clear_calls();
    for (tool, arguments) in [
        ("type_text", json!({"ref":"@s1:e2","text":"hunter2"})),
        ("set_value", json!({"ref":"@s1:e2","value":"hunter2"})),
        (
            "perform_secondary_action",
            json!({"ref":"@s1:e2","action":"clear"}),
        ),
    ] {
        assert_eq!(
            refused(fx.call(tool, arguments, Grant::Act).await),
            guard::PASSWORD_FIELD_REFUSAL,
            "{tool}"
        );
    }
    assert!(fx.calls().is_empty());
    let typed = fx
        .call(
            "type_text",
            json!({"ref":"@s1:e3","text":"report"}),
            Grant::Act,
        )
        .await
        .unwrap();
    assert!(text(&typed).starts_with("Typed the text."));
    assert_eq!(fx.calls(), ["type --session=task-1 --stdin -- @s1:e3"]);
}

#[tokio::test]
async fn secure_keyboard_input_stops_keys_and_text() {
    let mut fx = Fixture::new(Platform::MacOs);
    fx.observe("TextEdit").await;
    fx.probe.secure.store(true, Ordering::SeqCst);
    for (tool, arguments) in [
        ("type_text", json!({"ref":"@s1:e3","text":"x"})),
        ("set_value", json!({"ref":"@s1:e3","value":"x"})),
        ("press_key", json!({"app":"TextEdit","key":"return"})),
    ] {
        assert_eq!(
            refused(fx.call(tool, arguments, Grant::Act).await),
            guard::SECURE_INPUT_REFUSAL,
            "{tool}"
        );
    }
    assert!(fx
        .call("click", json!({"ref":"@s1:e1"}), Grant::Act)
        .await
        .is_ok());
}

#[tokio::test]
async fn terminals_accept_looks_and_clicks_but_never_keys_text_or_drops() {
    let mut fx = Fixture::new(Platform::MacOs);
    fx.observe("Terminal").await;
    for (tool, arguments) in [
        ("type_text", json!({"ref":"@s1:e3","text":"rm -rf ~"})),
        ("set_value", json!({"ref":"@s1:e3","value":"x"})),
        ("press_key", json!({"app":"Terminal","key":"return"})),
        ("drag", json!({"from_ref":"@s1:e1","to_ref":"@s1:e3"})),
    ] {
        assert_eq!(
            refused(fx.call(tool, arguments, Grant::Act).await),
            guard::TERMINAL_REFUSAL,
            "{tool}"
        );
    }
    assert!(fx
        .call("click", json!({"ref":"@s1:e1"}), Grant::Act)
        .await
        .is_ok());
}

#[tokio::test]
async fn privacy_settings_can_be_read_but_not_changed() {
    let mut fx = Fixture::new(Platform::MacOs);
    fx.observe("System Settings").await;
    assert_eq!(
        refused(fx.call("click", json!({"ref":"@s1:e1"}), Grant::Act).await),
        guard::PRIVACY_SETTINGS_REFUSAL
    );
    assert!(fx
        .call(
            "perform_secondary_action",
            json!({"ref":"@s1:e1","action":"expand"}),
            Grant::Act
        )
        .await
        .is_ok());
    // Another pane of the same app is not refused.
    fx.set(
        "list-windows",
        &envelope(
            "list-windows",
            json!([window(
                "ws",
                400,
                "System Settings",
                "Appearance",
                (20.0, 20.0, 600.0, 400.0)
            )]),
        ),
    );
    assert!(fx
        .call("click", json!({"ref":"@s1:e1"}), Grant::Act)
        .await
        .is_ok());
}

#[tokio::test]
async fn coordinates_follow_the_latest_screenshot_and_refuse_a_moved_window() {
    let mut fx = Fixture::new(Platform::MacOs);
    let before = refused(
        fx.call("click", json!({"app":"TextEdit","x":10,"y":10}), Grant::Act)
            .await,
    );
    assert!(before.contains("computer_use_stale_state"), "{before}");
    let state = fx
        .call(
            "get_app_state",
            json!({"app":"TextEdit","screenshot":true}),
            Grant::Observe,
        )
        .await
        .unwrap();
    let image = state
        .content
        .iter()
        .find_map(|c| c.as_image())
        .expect("a screenshot");
    assert_eq!(image.mime_type, "image/png");
    // The 2000 by 1000 capture is scaled to a long side of 1568.
    let shots: Vec<_> = std::fs::read_dir(fx.dir.path().join("runs"))
        .unwrap()
        .collect();
    assert_eq!(shots.len(), 1, "one chat folder");
    fx.clear_calls();
    let clicked = fx
        .call(
            "click",
            json!({"app":"TextEdit","x":784,"y":392}),
            Grant::Act,
        )
        .await
        .unwrap();
    assert!(text(&clicked).starts_with("Clicked at that point."));
    let calls = fx.calls();
    let click = calls
        .iter()
        .find(|call| call.starts_with("mouse-click"))
        .unwrap();
    // Pixel (784, 392) of a 1568 by 784 image of a 1000 by 500 point window at (100, 50).
    assert_eq!(
        click,
        "mouse-click --headed --session=task-1 --xy=600,300 --button=left --count=1"
    );
    let outside = refused(
        fx.call(
            "click",
            json!({"app":"TextEdit","x":1600,"y":10}),
            Grant::Act,
        )
        .await,
    );
    assert!(outside.contains("outside the latest screenshot"));
    fx.set(
        "list-windows",
        &envelope(
            "list-windows",
            json!([window(
                "w1",
                100,
                "TextEdit",
                "Untitled",
                (300.0, 50.0, 1000.0, 500.0)
            )]),
        ),
    );
    let moved = refused(
        fx.call("click", json!({"app":"TextEdit","x":10,"y":10}), Grant::Act)
            .await,
    );
    assert!(moved.contains("the window moved"), "{moved}");
}

#[tokio::test]
async fn a_display_capture_is_refused_while_a_biorouter_window_is_on_it() {
    let mut fx = Fixture::new(Platform::MacOs);
    let covered = refused(
        fx.call("screen_capture", json!({"display":0}), Grant::Observe)
            .await,
    );
    assert!(covered.contains("a Biorouter window is on that display"));
    let side = fx
        .call("screen_capture", json!({"display":1}), Grant::Observe)
        .await
        .unwrap();
    assert!(side.content.iter().any(|c| c.as_image().is_some()));
    assert!(fx
        .calls()
        .iter()
        .any(|call| call.starts_with("screenshot --session=task-1 --screen=1 -- ")));
    let missing = refused(
        fx.call("screen_capture", json!({"display":5}), Grant::Observe)
            .await,
    );
    assert!(missing.contains("computer_use_display_not_found"));
}

#[tokio::test]
async fn drag_runs_one_headed_gesture_and_reports_an_unaccepted_drop() {
    let mut fx = Fixture::new(Platform::MacOs);
    fx.observe("TextEdit").await;
    fx.set(
        "drag",
        &envelope("drag", json!({"action":"drag","disposition":{"delivery":"delivered_unverified","retry":"unsafe"},
                                 "steps":[{"label":"pointer_drag","outcome":"succeeded","mechanism":"physical_synthetic"}]})),
    );
    fx.clear_calls();
    let dragged = fx
        .call(
            "drag",
            json!({"from_ref":"@s1:e1","to_ref":"@s1:e3"}),
            Grant::Act,
        )
        .await
        .unwrap();
    assert!(text(&dragged)
        .contains("Dragged and dropped. Observe the destination before saying it worked"));
    assert!(text(&dragged).contains("Delivered, but not verified"));
    assert_eq!(
        fx.calls(),
        ["drag --headed --session=task-1 --from=@s1:e1 --to=@s1:e3 --timeout-ms=15000"]
    );
    fx.set(
        "drag",
        &json!({"version":"2.6","ok":false,"command":"drag","error":{"code":"TIMEOUT","message":"wait timed out","details":{"kind":"wait_timeout"},"disposition":{"delivery":"delivered_unverified","retry":"unsafe"}}}).to_string(),
    );
    let unaccepted = fx
        .call(
            "drag",
            json!({"from_ref":"@s1:e1","to_ref":"@s1:e3","verify":"textfield:Report"}),
            Grant::Act,
        )
        .await
        .unwrap();
    assert_eq!(unaccepted.is_error, Some(true));
    assert!(text(&unaccepted).contains("did not appear at the destination within 10 seconds"));
    // The failed verification makes the next action observe first.
    let stale = refused(fx.call("click", json!({"ref":"@s1:e1"}), Grant::Act).await);
    assert!(stale.contains("computer_use_stale_reference"));
}

#[tokio::test]
async fn an_uncertain_outcome_forgets_the_app_until_it_is_observed_again() {
    let mut fx = Fixture::new(Platform::MacOs);
    fx.observe("TextEdit").await;
    fx.set(
        "click",
        &json!({"version":"2.6","ok":false,"command":"click","error":{"code":"ACTION_FAILED","message":"the app stopped answering","suggestion":"Retry with --headed","disposition":{"delivery":"delivery_uncertain","retry":"unsafe"}}}).to_string(),
    );
    let uncertain = fx
        .call("click", json!({"ref":"@s1:e1"}), Grant::Act)
        .await
        .unwrap();
    assert_eq!(uncertain.is_error, Some(true));
    let words = text(&uncertain);
    assert!(words.contains("do not repeat it"), "{words}");
    assert!(!words.to_lowercase().contains("headed"));
    let stale = refused(fx.call("click", json!({"ref":"@s1:e1"}), Grant::Act).await);
    assert!(stale.contains("computer_use_stale_reference"));
    let keys = refused(
        fx.call(
            "press_key",
            json!({"app":"TextEdit","key":"return"}),
            Grant::Act,
        )
        .await,
    );
    assert!(keys.contains("computer_use_stale_state"));
    fx.observe("TextEdit").await;
    assert!(fx
        .call(
            "press_key",
            json!({"app":"TextEdit","key":"cmd+s"}),
            Grant::Act
        )
        .await
        .is_ok());
    assert!(fx
        .calls()
        .iter()
        .any(|call| call == "press --session=task-1 --app=TextEdit -- cmd+s"));
}

#[tokio::test]
async fn the_task_folder_is_private_and_goes_with_the_task() {
    let mut fx = Fixture::new(Platform::MacOs);
    fx.observe("TextEdit").await;
    let home = PathBuf::from(
        std::fs::read_to_string(fx.engine().join("home"))
            .unwrap()
            .trim(),
    );
    assert!(home.starts_with(fx.dir.path().join("runs")));
    assert!(home.ends_with("task-1"));
    let mode = std::fs::metadata(&home).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
    assert_eq!(
        std::fs::read_to_string(home.join("owner")).unwrap(),
        std::process::id().to_string()
    );
    fx.observe("Finder").await;
    fx.call_in("chat", "task-2", "list_apps", json!({}), Grant::Observe)
        .await
        .unwrap();
    assert!(
        !home.exists(),
        "a new generation deletes the old task's folder"
    );
}

#[tokio::test]
async fn stop_ends_the_running_command_and_deletes_the_task_folder() {
    let fx = Fixture::new(Platform::MacOs);
    let engine = fx.engine();
    let runtime = Arc::new(tokio::sync::Mutex::new(fx.runtime));
    let chat = uuid::Uuid::new_v4().to_string();
    {
        let mut session = runtime.lock().await;
        session
            .call(
                &chat,
                "task-1",
                "get_app_state",
                json!({"app":"TextEdit"}),
                Grant::Act,
                CancellationToken::new(),
            )
            .await
            .unwrap();
    }
    let home = PathBuf::from(std::fs::read_to_string(engine.join("home")).unwrap().trim());
    std::fs::write(engine.join("block-drag"), "").unwrap();
    let token = CancellationToken::new();
    register_session(&chat, "task-1", &runtime, token.clone());
    let running = {
        let runtime = runtime.clone();
        let chat = chat.clone();
        tokio::spawn(async move {
            let mut session = runtime.lock().await;
            session
                .call(
                    &chat,
                    "task-1",
                    "drag",
                    json!({"from_ref":"@s1:e1","to_ref":"@s1:e3"}),
                    Grant::Act,
                    token,
                )
                .await
        })
    };
    for _ in 0..500 {
        if engine.join("blocked.pid").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        engine.join("blocked.pid").exists(),
        "the drag never started"
    );
    stop_session(&chat, "old-task").await;
    assert!(
        !running.is_finished(),
        "Stop of another generation leaves this one running"
    );
    stop_session(&chat, "task-1").await;
    let error = running.await.unwrap().unwrap_err();
    assert!(
        error.message.contains("computer_use_cancelled"),
        "{}",
        error.message
    );
    assert_eq!(
        std::fs::read_to_string(engine.join("released"))
            .unwrap()
            .trim(),
        "released",
        "SIGTERM reached the engine first, so its release guard ran"
    );
    assert!(!home.exists(), "Stop deletes the task folder");
    assert!(runtime.lock().await.task.is_none());
}

#[tokio::test]
async fn engine_errors_carry_biorouters_advice_and_the_disposition() {
    let mut fx = Fixture::new(Platform::MacOs);
    fx.set(
        "snapshot",
        &json!({"version":"2.6","ok":false,"command":"snapshot","error":{"code":"PERM_DENIED","message":"Accessibility permission is not granted","suggestion":"Run agent-desktop permissions --request","disposition":{"delivery":"not_delivered","retry":"safe"}}}).to_string(),
    );
    let denied = fx
        .call("get_app_state", json!({"app":"TextEdit"}), Grant::Observe)
        .await
        .unwrap();
    assert_eq!(denied.is_error, Some(true));
    let words = text(&denied);
    assert!(words.starts_with("PERM_DENIED: Accessibility permission is not granted."));
    assert!(words.contains("allow Biorouter Copilot in System Settings"));
    assert!(!words.contains("agent-desktop"));
}

#[tokio::test]
async fn wayland_and_linux_rules_apply_before_anything_runs() {
    let mut fx = Fixture::new(Platform::LinuxWayland);
    fx.observe("TextEdit").await;
    fx.clear_calls();
    let drag = refused(
        fx.call(
            "drag",
            json!({"from_ref":"@s1:e1","to_ref":"@s1:e3"}),
            Grant::Act,
        )
        .await,
    );
    assert_eq!(drag, plan::UNSUPPORTED);
    assert!(!fx.calls().iter().any(|call| call.starts_with("drag")));
    let mut linux = Fixture::new(Platform::LinuxX11);
    let open = refused(
        linux
            .call(
                "open_app",
                json!({"app":"definitely-not-installed-app-0x1"}),
                Grant::Act,
            )
            .await,
    );
    assert!(open.contains("desktop entry"), "{open}");
    assert!(!linux.calls().iter().any(|call| call.starts_with("launch")));
}

#[tokio::test]
async fn open_app_launches_by_name_and_releases_nothing_but_the_app() {
    let mut fx = Fixture::new(Platform::MacOs);
    fx.set(
        "launch",
        &envelope(
            "launch",
            json!({"app":"Notes","pid":700,"window":{"id":"wn","title":"Notes"}}),
        ),
    );
    let opened = fx
        .call("open_app", json!({"app":"Notes"}), Grant::Act)
        .await
        .unwrap();
    assert!(text(&opened).contains("Opened and brought forward"));
    assert!(fx
        .calls()
        .iter()
        .any(|call| call == "launch --session=task-1 --activate --timeout=30000 -- Notes"));
    fx.set(
        "launch",
        &envelope("launch", json!({"app":"Sneaky","pid":200})),
    );
    assert_eq!(
        refused(
            fx.call("open_app", json!({"app":"Sneaky"}), Grant::Act)
                .await
        ),
        guard::BIOROUTER_REFUSAL,
        "an app that turns out to be Biorouter is not reported as opened"
    );
}

#[tokio::test]
async fn wait_for_reads_a_snapshot_or_waits_on_text() {
    let mut fx = Fixture::new(Platform::MacOs);
    let appeared = fx
        .call(
            "wait_for",
            json!({"app":"TextEdit","element":"button:Save","ms":2000}),
            Grant::Observe,
        )
        .await
        .unwrap();
    assert!(text(&appeared).starts_with("It appeared."));
    let snapshot = fx
        .calls()
        .into_iter()
        .find(|call| call.starts_with("snapshot"))
        .unwrap();
    assert!(
        snapshot.contains("--wait-for=button:Save --wait-timeout=2000"),
        "{snapshot}"
    );
    // The snapshot's references are usable afterwards.
    assert!(fx
        .call("click", json!({"ref":"@s1:e1"}), Grant::Act)
        .await
        .is_ok());
    let waited = fx
        .call(
            "wait_for",
            json!({"app":"TextEdit","text":"Saved"}),
            Grant::Observe,
        )
        .await
        .unwrap();
    assert_eq!(text(&waited), "The text appeared.");
    assert!(fx.calls().iter().any(|call| call
        .starts_with("wait --session=task-1 --text=Saved --app=TextEdit --timeout=10000")));
    let two = refused(
        fx.call(
            "wait_for",
            json!({"app":"TextEdit","text":"a","window_title":"b"}),
            Grant::Observe,
        )
        .await,
    );
    assert!(two.contains("Give one of"));
}

#[test]
fn the_probe_bound_exceeds_the_bound_of_the_command_it_supervises() {
    assert!(PROBE_BOUND > plan::OBSERVE + runner::OUTER_GRACE + Duration::from_secs(5));
}

#[test]
fn version_floors_compare_numerically() {
    assert!(version_at_least("2.52.0-1ubuntu1", ATSPI_MINIMUM));
    assert!(version_at_least("2.54.1", ATSPI_MINIMUM));
    assert!(!version_at_least("2.44.0-3", ATSPI_MINIMUM));
    assert!(!version_at_least("2.46.0", ATSPI_MINIMUM));
    assert!(!version_at_least("", ATSPI_MINIMUM));
}

#[tokio::test]
async fn stop_of_an_idle_task_deletes_its_folder_and_leaves_a_newer_generation_alone() {
    let fx = Fixture::new(Platform::MacOs);
    let engine = fx.engine();
    let runtime = Arc::new(tokio::sync::Mutex::new(fx.runtime));
    let chat = uuid::Uuid::new_v4().to_string();
    for _ in 0..3 {
        let token = CancellationToken::new();
        register_session(&chat, "task-1", &runtime, token.clone());
        let mut session = runtime.lock().await;
        session
            .call(
                &chat,
                "task-1",
                "list_apps",
                json!({}),
                Grant::Observe,
                token,
            )
            .await
            .unwrap();
    }
    let registered = SESSION_RUNTIMES
        .lock()
        .unwrap()
        .get(&chat)
        .map(Vec::len)
        .unwrap_or_default();
    assert_eq!(
        registered, 1,
        "one registration per runtime, however many calls"
    );
    let home = PathBuf::from(std::fs::read_to_string(engine.join("home")).unwrap().trim());
    assert!(home.exists());
    stop_session(&chat, "task-0").await;
    assert!(
        home.exists(),
        "Stop of another generation leaves this task alone"
    );
    stop_session(&chat, "task-1").await;
    assert!(!home.exists(), "Stop deletes an idle task's folder");
}
