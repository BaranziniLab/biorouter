use anyhow::{anyhow, Result};

use crate::agents::effort::ReasoningEffort;
use crate::agents::types::SessionConfig;
use crate::context_mgmt::compact_messages;
use crate::conversation::message::{Message, SystemNotificationType};
use crate::workflow::build_workflow::build_workflow_from_template_with_positional_params;

use super::Agent;

pub const COMPACT_TRIGGERS: &[&str] =
    &["/compact", "Please compact this conversation", "/summarize"];

pub struct CommandDef {
    pub name: &'static str,
    pub description: &'static str,
}

static COMMANDS: &[CommandDef] = &[
    CommandDef {
        name: "compact",
        description: "Compact the chat history",
    },
    CommandDef {
        name: "clear",
        description: "Clear the chat history",
    },
    CommandDef {
        name: "goal",
        description: "Keep working until a condition is met (/goal <condition>, /goal clear)",
    },
    CommandDef {
        name: "loop",
        description: "Run a prompt on an interval (/loop 5m <prompt>, /loop stop <id>)",
    },
    CommandDef {
        name: "schedule",
        description: "Schedule a recurring prompt (/schedule @daily <prompt>, /schedule list)",
    },
    CommandDef {
        name: "effort",
        description: "Set how hard to think (/effort quick|normal|deep)",
    },
];

pub fn list_commands() -> &'static [CommandDef] {
    COMMANDS
}

fn extension_command_guidance(params: &str) -> String {
    match super::extension_manager::resolve_bundled_extension(params.trim()) {
        Some(target) => format!("/extend is not a command. Select {} with /ext:{} followed by your request. Existing tool approvals still apply.", target.display_name(), target.key()),
        None => "/extend is not a command. Use /ext:<extension-id> followed by your request, or choose an extension from the slash menu. For Biorouter Copilot, use /ext:computercontroller.".to_string(),
    }
}

fn workflow_parameter_guidance(command: &str, path: &std::path::Path, names: &[String]) -> String {
    format!(
        "The /{command} workflow requires {} parameters: {}.\n\n\
        Slash command workflows only support 1 parameter.\n\n\
        **To use this workflow:**\n\
        • **CLI:** Pass the workflow file at {} to `biorouter run --workflow` and supply a `--params KEY=VALUE` argument for each parameter.\n\
        • **Desktop:** Launch from the workflows sidebar to fill in parameters",
        names.len(), names.join(", "), path.display()
    )
}

impl Agent {
    pub async fn execute_command(
        &self,
        message_text: &str,
        session_config: &SessionConfig,
    ) -> Result<Option<Message>> {
        let session_id = session_config.id.as_str();
        let mut trimmed = message_text.trim().to_string();

        if COMPACT_TRIGGERS.contains(&trimmed.as_str()) {
            trimmed = COMPACT_TRIGGERS[0].to_string();
        }

        if !trimmed.starts_with('/') {
            return Ok(None);
        }

        let command_str = trimmed.strip_prefix('/').unwrap_or(&trimmed);
        let (command, params_str) = command_str
            .split_once(char::is_whitespace)
            .map(|(cmd, p)| (cmd, p.trim()))
            .unwrap_or((command_str, ""));

        if ["skill:", "ext:", "kb:", "skill(", "ext(", "kb("]
            .iter()
            .any(|prefix| command.starts_with(prefix))
        {
            return Ok(None);
        }

        match command {
            "compact" => self.handle_compact_command(session_config).await,
            "clear" => self.handle_clear_command(session_id).await,
            "goal" => self.handle_goal_command(params_str, session_id).await,
            "loop" => self.handle_loop_command(params_str, session_id).await,
            "schedule" => self.handle_schedule_command(params_str, session_id).await,
            "effort" => self.handle_effort_command(params_str, session_id).await,
            "knowledge" => Ok(Some(Message::assistant().with_system_notification(SystemNotificationType::InlineMessage, "Select Knowledge with /ext:knowledge followed by your request, or choose a knowledge base with /kb:<id>."))),
            "extend" => Ok(Some(Message::assistant().with_system_notification(
                SystemNotificationType::InlineMessage,
                extension_command_guidance(params_str),
            ))),
            _ => {
                self.handle_workflow_command(command, params_str, session_id)
                    .await
            }
        }
    }

    async fn handle_compact_command(
        &self,
        session_config: &SessionConfig,
    ) -> Result<Option<Message>> {
        let session_id = session_config.id.as_str();
        let manager = self.config.session_manager.clone();
        let (session, basis) = manager.snapshot_for_rewrite(session_id).await?;
        let stored_conversation = session
            .conversation
            .ok_or_else(|| anyhow!("Session has no conversation"))?;
        let conversation = stored_conversation.clone();

        self.fire_compaction_hook(
            crate::hooks::HookEvent::PreCompact,
            session_id,
            &session.working_dir,
            "manual",
            None,
        );

        let usage_event_key = uuid::Uuid::new_v4().to_string();
        let (compacted_conversation, summarization_usage) = compact_messages(
            self.provider().await?.as_ref(),
            &conversation,
            true, // is_manual_compact
        )
        .await?;

        // Same freshness discipline as the two in-turn compaction sites: a
        // message another writer appended while the summarizer ran is carried
        // over rather than deleted by the DELETE+reinsert.
        let (outcome, _stored) = manager
            .replace_conversation_preserving_tail(
                session_id,
                &compacted_conversation,
                basis,
                &stored_conversation,
            )
            .await?;

        if !outcome.stored() {
            tracing::warn!(
                "Manual compaction skipped for session {session_id} ({outcome:?}); the \
                 history changed while it was being summarized"
            );
            // The round-trip was spent, so bill it — but not as a compaction:
            // it did not replace the context. No PostCompact either, since
            // nothing was compacted.
            self.update_session_metrics(
                session_config,
                &summarization_usage,
                false,
                &usage_event_key,
            )
            .await?;
            // User-initiated and trivially re-runnable, so say so plainly
            // instead of silently doing nothing.
            return Ok(Some(Message::assistant().with_system_notification(
                SystemNotificationType::InlineMessage,
                "Compaction skipped: this chat changed while it was being \
                 summarized, so compacting now would discard the new messages. \
                 Run /compact again.",
            )));
        }

        self.fire_compaction_hook(
            crate::hooks::HookEvent::PostCompact,
            session_id,
            &session.working_dir,
            "manual",
            None,
        );

        // Without this, session.total_tokens stays at the pre-compact value:
        // the UI gauge keeps reading the old number, and the next reply's
        // check_if_compaction_needed re-triggers a full LLM summarization.
        self.update_session_metrics(session_config, &summarization_usage, true, &usage_event_key)
            .await?;

        Ok(Some(Message::assistant().with_system_notification(
            SystemNotificationType::InlineMessage,
            "Compaction complete",
        )))
    }

    /// BR-63: `/effort quick|normal|deep` — the slash-flag half of the
    /// reasoning-effort control (the GUI composer toggle is the other half, and
    /// it wins for the turn it rides on). Sticky for the session; `/effort` with
    /// no argument reports the current level.
    async fn handle_effort_command(
        &self,
        params_str: &str,
        session_id: &str,
    ) -> Result<Option<Message>> {
        let current = self.reasoning_effort(session_id).await;

        if params_str.is_empty() {
            return Ok(Some(Message::assistant().with_system_notification(
                SystemNotificationType::InlineMessage,
                format!(
                    "Reasoning effort: {}. Change it with /effort quick|normal|deep.",
                    current.describe()
                ),
            )));
        }

        let Some(effort) = ReasoningEffort::parse(params_str) else {
            return Ok(Some(Message::assistant().with_system_notification(
                SystemNotificationType::InlineMessage,
                format!(
                    "Unknown effort '{params_str}'. Use /effort quick, /effort normal, or /effort deep."
                ),
            )));
        };

        self.set_reasoning_effort(session_id, effort).await;

        Ok(Some(Message::assistant().with_system_notification(
            SystemNotificationType::InlineMessage,
            format!("Reasoning effort set to {}", effort.describe()),
        )))
    }

    async fn handle_clear_command(&self, session_id: &str) -> Result<Option<Message>> {
        use crate::conversation::Conversation;

        let manager = self.config.session_manager.clone();
        manager
            .replace_conversation(session_id, &Conversation::default())
            .await?;

        manager
            .update(session_id)
            .total_tokens(Some(0))
            .input_tokens(Some(0))
            .output_tokens(Some(0))
            .apply()
            .await?;

        Ok(Some(Message::assistant().with_system_notification(
            SystemNotificationType::InlineMessage,
            "Chat cleared",
        )))
    }

    async fn handle_workflow_command(
        &self,
        command: &str,
        params_str: &str,
        _session_id: &str,
    ) -> Result<Option<Message>> {
        let workflow_path = match crate::slash_commands::get_workflow_for_command(command) {
            Some(path) => path,
            None => return Ok(None),
        };

        if crate::slash_commands::is_reserved_workflow_command(command) {
            return Err(anyhow!("/{command} is reserved by Biorouter. Rename this workflow's slash-command binding."));
        }
        if !workflow_path.is_file() {
            return Err(anyhow!("The /{command} workflow file is unavailable: {}. Restore the file or update this command's workflow binding.", workflow_path.display()));
        }

        let workflow_content = tokio::fs::read_to_string(&workflow_path)
            .await
            .map_err(|e| anyhow!("Failed to read workflow file: {}", e))?;

        let workflow_dir = workflow_path
            .parent()
            .ok_or_else(|| anyhow!("Workflow path has no parent directory"))?;

        let workflow_dir_str = workflow_dir.display().to_string();
        let validation_result =
            crate::workflow::validate_workflow::validate_workflow_template_from_content(
                &workflow_content,
                Some(workflow_dir_str),
            )
            .map_err(|e| anyhow!("Failed to parse workflow: {}", e))?;

        let param_values: Vec<String> = if params_str.is_empty() {
            vec![]
        } else {
            let params_without_default = validation_result
                .parameters
                .as_ref()
                .map(|params| params.iter().filter(|p| p.default.is_none()).count())
                .unwrap_or(0);

            if params_without_default <= 1 {
                vec![params_str.to_string()]
            } else {
                let param_names: Vec<String> = validation_result
                    .parameters
                    .as_ref()
                    .map(|params| {
                        params
                            .iter()
                            .filter(|p| p.default.is_none())
                            .map(|p| p.key.clone())
                            .collect()
                    })
                    .unwrap_or_default();

                return Err(anyhow!(workflow_parameter_guidance(
                    command,
                    &workflow_path,
                    &param_names
                )));
            }
        };

        let param_values_len = param_values.len();

        let workflow = match build_workflow_from_template_with_positional_params(
            workflow_content,
            workflow_dir,
            param_values,
            None::<fn(&str, &str) -> Result<String>>,
        ) {
            Ok(workflow) => workflow,
            Err(crate::workflow::build_workflow::WorkflowError::MissingParams { parameters }) => {
                return Ok(Some(Message::assistant().with_text(format!(
                    "Workflow requires {} parameter(s): {}. Provided: {}",
                    parameters.len(),
                    parameters.join(", "),
                    param_values_len
                ))));
            }
            Err(e) => return Err(anyhow!("Failed to build workflow: {}", e)),
        };

        self.apply_workflow_components(
            workflow.sub_workflows.clone(),
            workflow.response.clone(),
            true,
        )
        .await;

        let prompt = [workflow.instructions.as_deref(), workflow.prompt.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n\n");

        Ok(Some(Message::user().with_text(prompt)))
    }
}

#[cfg(test)]
mod slash_command_audit_tests {
    use super::*;

    #[test]
    fn legacy_extend_explains_the_current_name_and_marker() {
        let guidance = extension_command_guidance("computer controller");
        assert!(guidance.contains("Biorouter Copilot"));
        assert!(guidance.contains("/ext:computercontroller"));
        assert!(guidance.contains("approvals still apply"));
        assert_eq!(guidance, extension_command_guidance("Biorouter Copilot"));
    }

    #[test]
    fn unknown_extend_targets_receive_actionable_guidance() {
        let guidance = extension_command_guidance("unconfigured-extension");
        assert!(guidance.contains("/ext:<extension-id>"));
        assert!(guidance.contains("slash menu"));
    }

    #[test]
    fn multi_parameter_guidance_uses_the_actual_file_not_the_slash_alias() {
        let path = std::path::Path::new("/tmp/workflows/a 'quoted' workflow.yaml");
        let guidance =
            workflow_parameter_guidance("review", path, &["input".into(), "output".into()]);
        assert!(guidance.contains(&path.display().to_string()));
        assert!(guidance.contains("--params KEY=VALUE"));
        assert!(!guidance.contains("--workflow review"));
    }

    #[tokio::test]
    async fn legacy_commands_are_handled_without_a_provider() {
        let agent = Agent::new();
        let config: SessionConfig =
            serde_json::from_value(serde_json::json!({"id": "slash-command-audit"})).unwrap();
        for command in [
            "/extend computer controller",
            "/extend\tcomputer controller",
            "/extend\ncomputer controller",
            "/knowledge",
        ] {
            assert!(agent
                .execute_command(command, &config)
                .await
                .unwrap()
                .is_some());
        }
        assert!(agent
            .execute_command("/effort\tdeep", &config)
            .await
            .unwrap()
            .is_some());
        assert_eq!(
            agent.reasoning_effort(&config.id).await,
            ReasoningEffort::Deep
        );
        assert!(agent
            .execute_command("/effort\nquick", &config)
            .await
            .unwrap()
            .is_some());
        assert_eq!(
            agent.reasoning_effort(&config.id).await,
            ReasoningEffort::Quick
        );
        for resource in [
            "/ext:computercontroller help",
            "/ext(developer) help",
            "/skill(rna) help",
            "/kb(research) help",
        ] {
            assert!(agent
                .execute_command(resource, &config)
                .await
                .unwrap()
                .is_none());
        }
    }
}
