//! The permission decision for every tool call a Code Execution script makes
//! (QA finding F7, 2026-09-10).

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use rmcp::model::{CallToolRequestParams, CallToolResult, JsonObject, RawContent};
    use rmcp::object;
    use tokio_util::sync::CancellationToken;

    use crate::action_required_manager::ActionRequiredManager;
    use crate::agents::extension::ExtensionConfig;
    use crate::agents::{Agent, AgentConfig};
    use crate::config::permission::{PermissionLevel, PermissionManager};
    use crate::config::BioRouterMode;
    use crate::conversation::message::{ActionRequiredData, MessageContent};
    use crate::pending_user_action::DecisionAuthority;
    use crate::permission::permission_confirmation::PrincipalType;
    use crate::permission::{Permission, PermissionConfirmation};
    use crate::session::session_manager::SessionType;
    use crate::session::{Session, SessionManager};

    const EXECUTE_CODE: &str = "code_execution__execute_code";
    const SHELL: &str = "developer__shell";

    /// A real agent — its own inspector stack, its own permission table — with
    /// the two extensions the QA run used: `developer` and `code_execution`.
    ///
    /// The permission table is private to the fixture and pre-seeded exactly as
    /// the QA sandbox's was in the one entry that matters: the SCRIPT is always
    /// allowed. Everything the script calls must then stand on its own name.
    struct Fixture {
        agent: Arc<Agent>,
        session: Session,
        permissions: Arc<PermissionManager>,
        dir: tempfile::TempDir,
    }

    async fn fixture(mode: BioRouterMode) -> Fixture {
        let dir = tempfile::TempDir::new().expect("a scratch directory");
        let sessions = Arc::new(SessionManager::new(dir.path().join("sessions")));
        let permissions = Arc::new(PermissionManager::new(dir.path().join("config")));
        let agent = Arc::new(Agent::with_config(AgentConfig::new(
            Arc::clone(&sessions),
            Arc::clone(&permissions),
            None,
            mode,
        )));
        agent
            .add_extension(ExtensionConfig::Builtin {
                name: "developer".into(),
                description: "developer".into(),
                display_name: Some("Developer".into()),
                timeout: Some(300),
                bundled: Some(true),
                available_tools: vec![],
            })
            .await
            .expect("enable developer");
        agent
            .add_extension(ExtensionConfig::Platform {
                name: "code_execution".into(),
                description: "code execution".into(),
                bundled: Some(true),
                available_tools: vec![],
            })
            .await
            .expect("enable code_execution");
        let session = sessions
            .create_session(
                dir.path().to_path_buf(),
                "script gate".into(),
                SessionType::User,
            )
            .await
            .expect("a session");
        permissions.update_user_permission(EXECUTE_CODE, PermissionLevel::AlwaysAllow);
        // A card left over from another test that minted the same session id
        // (they are `YYYYMMDD_N` per database) must not be read as ours.
        ActionRequiredManager::global().drain_requests(&session.id);
        Fixture {
            agent,
            session,
            permissions,
            dir,
        }
    }

    fn text_of(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|content| match &content.raw {
                RawContent::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Dispatch `execute_code` through [`Agent::dispatch_tool_call`] — the one
    /// function every model-initiated call reaches, approved or not — and drive
    /// the script's body on its own task, the way the reply loop's batch does.
    ///
    /// ⚠ Deliberately NOT `ExtensionManager::dispatch_tool_call`: that is the
    /// path a PERSON drives (`POST /agent/call_tool`), and it is the agent's
    /// dispatch that has to carry the script's judge down to its calls.
    async fn run_script(
        f: &Fixture,
        code: &str,
        cancel: CancellationToken,
    ) -> tokio::task::JoinHandle<(bool, String)> {
        let call = CallToolRequestParams {
            task: None,
            meta: None,
            name: EXECUTE_CODE.into(),
            arguments: Some(object!({ "code": code })),
        };
        let (_, dispatched) = f
            .agent
            .dispatch_tool_call(call, "outer-execute-code".into(), Some(cancel), &f.session)
            .await;
        let dispatched = dispatched.expect("execute_code dispatches");
        tokio::spawn(async move {
            let result = dispatched
                .result
                .await
                .expect("execute_code returns a result");
            (result.is_error.unwrap_or(false), text_of(&result))
        })
    }

    struct Card {
        id: String,
        tool_name: String,
        arguments: JsonObject,
        prompt: Option<String>,
    }

    /// The next approval card published for this session, or `None` if the
    /// script finished without one.
    ///
    /// Raced against the script itself rather than a bare timeout: a script
    /// that runs its call without asking finishes, and waiting out a clock
    /// after that proves nothing but patience.
    async fn card_or_completion(
        session_id: &str,
        script: &mut tokio::task::JoinHandle<(bool, String)>,
    ) -> Result<Card, (bool, String)> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            for message in ActionRequiredManager::global().drain_requests(session_id) {
                for content in &message.content {
                    let MessageContent::ActionRequired(action) = content else {
                        continue;
                    };
                    if let ActionRequiredData::ToolConfirmation {
                        id,
                        tool_name,
                        arguments,
                        prompt,
                        ..
                    } = &action.data
                    {
                        return Ok(Card {
                            id: id.clone(),
                            tool_name: tool_name.clone(),
                            arguments: arguments.clone(),
                            prompt: prompt.clone(),
                        });
                    }
                }
            }
            if script.is_finished() {
                return Err(script.await.expect("the script task completes"));
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "neither an approval card nor a finished script within 60s"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn answer(f: &Fixture, card: &Card, permission: Permission) {
        let outcome = f
            .agent
            .handle_confirmation_for_session(
                &f.session.id,
                card.id.clone(),
                PermissionConfirmation {
                    principal_type: PrincipalType::Tool,
                    permission,
                },
                DecisionAuthority::unproven(),
            )
            .await;
        assert_eq!(
            outcome,
            crate::agents::ConfirmationOutcome::Delivered,
            "the card's decision must reach the parked call"
        );
    }

    async fn finish(script: tokio::task::JoinHandle<(bool, String)>) -> (bool, String) {
        tokio::time::timeout(Duration::from_secs(60), script)
            .await
            .expect("the script finishes once answered")
            .expect("the script task completes")
    }

    /// F7's headline, measured the way the QA run measured it: Manual mode, the
    /// script itself on the user's always-allow list, and a shell call inside
    /// it. The shell call is not on that list, so it must be put to the user —
    /// on a card that names `developer__shell` and carries the command — and it
    /// must run once they allow it.
    #[tokio::test]
    #[serial_test::serial]
    async fn manual_mode_asks_for_a_scripts_shell_call_under_the_inner_tools_name() {
        let f = fixture(BioRouterMode::Approve).await;
        let mut script = run_script(
            &f,
            r#"import { shell } from "developer";
               record_result(shell({ command: "echo SCRIPT-GATE-ALLOWED" }));"#,
            CancellationToken::new(),
        )
        .await;

        let card = match card_or_completion(&f.session.id, &mut script).await {
            Ok(card) => card,
            Err((_, output)) => panic!(
                "the script's developer__shell call ran with no approval card, because the \
                 always-allowed script vouched for it: {output}"
            ),
        };
        assert_eq!(
            card.tool_name, SHELL,
            "the card must name the tool the script called, not the script"
        );
        assert_eq!(
            card.arguments.get("command").and_then(|v| v.as_str()),
            Some("echo SCRIPT-GATE-ALLOWED"),
            "the card must carry the call's own evaluated arguments"
        );

        answer(&f, &card, Permission::AllowOnce).await;
        let (is_error, output) = finish(script).await;
        assert!(!is_error, "an allowed call runs: {output}");
        assert!(
            output.contains("SCRIPT-GATE-ALLOWED"),
            "the allowed shell call's output reaches the script: {output}"
        );
    }

    /// The other answer on the same card: a denial comes back INTO the script
    /// as a tool error it can catch, and the script goes on — it is not ended
    /// silently, and the command never runs.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_denied_card_is_a_catchable_tool_error_and_the_script_goes_on() {
        let f = fixture(BioRouterMode::Approve).await;
        let marker = f.dir.path().join("denied-marker");
        let code = format!(
            r#"import {{ shell }} from "developer";
               let caught = null;
               try {{ shell({{ command: "touch '{marker}'" }}); }}
               catch (e) {{ caught = String(e); }}
               record_result({{ caught, continued: true }});"#,
            marker = marker.display()
        );
        let mut script = run_script(&f, &code, CancellationToken::new()).await;

        let card = match card_or_completion(&f.session.id, &mut script).await {
            Ok(card) => card,
            Err((_, output)) => {
                panic!("the script's developer__shell call ran with no approval card: {output}")
            }
        };
        assert_eq!(card.tool_name, SHELL);
        answer(&f, &card, Permission::DenyOnce).await;

        let (is_error, output) = finish(script).await;
        assert!(!is_error, "the script caught the refusal and finished: {output}");
        assert!(
            output.contains("\"continued\":true"),
            "the script must go on past a refused call: {output}"
        );
        assert!(
            output.contains(SHELL) && output.contains("declined"),
            "the error the script caught must name the refused tool and say why: {output}"
        );
        assert!(!marker.exists(), "a denied command must not have run");
    }

    /// `always_deny` is keyed by the INNER tool's name, exactly as it is for a
    /// direct call: the refusal needs no card, it is an error the script can
    /// catch, and the script's next call is judged on its own name.
    #[tokio::test]
    #[serial_test::serial]
    async fn always_deny_on_the_inner_tool_refuses_it_and_the_script_continues() {
        let f = fixture(BioRouterMode::Approve).await;
        f.permissions
            .update_user_permission(SHELL, PermissionLevel::NeverAllow);
        f.permissions
            .update_user_permission("developer__text_editor", PermissionLevel::AlwaysAllow);
        let marker = f.dir.path().join("never-marker");
        // The developer server's path jail is the process working directory
        // here, which `cargo test` sets to this crate's root — so the next call
        // reads a file that is certainly inside it.
        let code = format!(
            r#"import {{ shell, text_editor }} from "developer";
               let caught = null;
               try {{ shell({{ command: "touch '{marker}'" }}); }}
               catch (e) {{ caught = String(e); }}
               const after = text_editor({{ command: "view", path: "Cargo.toml" }});
               record_result({{ caught, after }});"#,
            marker = marker.display(),
        );
        let mut script = run_script(&f, &code, CancellationToken::new()).await;

        let (is_error, output) = match card_or_completion(&f.session.id, &mut script).await {
            Ok(card) => panic!(
                "a call the user always denies must be refused without a card, got one for {}",
                card.tool_name
            ),
            Err(finished) => finished,
        };
        assert!(!is_error, "the script caught the refusal and finished: {output}");
        assert!(
            output.contains(SHELL) && output.contains("declined"),
            "the always-denied call must come back into the script as a tool error: {output}"
        );
        assert!(
            output.contains("[package]"),
            "the script's next call must still run: {output}"
        );
        assert!(!marker.exists(), "an always-denied command must not have run");
    }

    /// Auto mode is unchanged: the same script call runs, and no card is
    /// raised for it.
    #[tokio::test]
    #[serial_test::serial]
    async fn auto_mode_runs_a_scripts_shell_call_with_no_card() {
        let f = fixture(BioRouterMode::Auto).await;
        let mut script = run_script(
            &f,
            r#"import { shell } from "developer";
               record_result(shell({ command: "echo SCRIPT-GATE-AUTO" }));"#,
            CancellationToken::new(),
        )
        .await;

        let (is_error, output) = match card_or_completion(&f.session.id, &mut script).await {
            Ok(card) => panic!("Auto mode raised a card for {}", card.tool_name),
            Err(finished) => finished,
        };
        assert!(!is_error, "{output}");
        assert!(output.contains("SCRIPT-GATE-AUTO"), "{output}");
    }
}
