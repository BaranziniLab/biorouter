use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::config::{Config, ConfigError};
use crate::workflow::Workflow;

const SLASH_COMMANDS_CONFIG_KEY: &str = "slash_commands";

/// Guards the once-per-process debug note about an unset `slash_commands` key.
static MISSING_KEY_LOGGED: AtomicBool = AtomicBool::new(false);
const REMOVED_SLASH_COMMANDS: &[&str] = &["prompt", "prompts"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashCommandMapping {
    pub command: String,
    pub workflow_path: String,
}

fn configured_commands() -> Vec<SlashCommandMapping> {
    commands_or_default(Config::global().get_param(SLASH_COMMANDS_CONFIG_KEY))
}

pub fn list_commands() -> Vec<SlashCommandMapping> {
    command_catalog(configured_commands())
}

fn command_catalog(mappings: Vec<SlashCommandMapping>) -> Vec<SlashCommandMapping> {
    let mut seen = std::collections::HashSet::new();
    mappings
        .into_iter()
        .map(|mut mapping| {
            mapping.command = mapping.command.trim_start_matches('/').to_lowercase();
            mapping
        })
        .filter(|mapping| {
            !mapping.command.is_empty()
                && !mapping.command.chars().any(char::is_whitespace)
                && !is_reserved_workflow_command(&mapping.command)
                && seen.insert(mapping.command.clone())
        })
        .collect()
}

/// Resolve the configured mappings, tolerating a config that has never defined
/// the key.
///
/// An absent optional key is the default state, not a warning condition — and
/// `list_commands` is called many times per session, so warning on it filled
/// the log with the same line 23 times (issue #49). It is recorded once per
/// process at `debug`; a *real* failure (unreadable or undeserializable config)
/// still warns every time.
fn commands_or_default(
    loaded: Result<Vec<SlashCommandMapping>, ConfigError>,
) -> Vec<SlashCommandMapping> {
    match loaded {
        Ok(commands) => commands,
        Err(ConfigError::NotFound(_)) => {
            if !MISSING_KEY_LOGGED.swap(true, Ordering::Relaxed) {
                debug!(
                    "No {} configured; using an empty list.",
                    SLASH_COMMANDS_CONFIG_KEY
                );
            }
            Vec::new()
        }
        Err(err) => {
            warn!(
                "Failed to load {}: {}. Falling back to empty list.",
                SLASH_COMMANDS_CONFIG_KEY, err
            );
            Vec::new()
        }
    }
}

pub fn is_removed_slash_command(command: &str) -> bool {
    let normalized = command.trim_start_matches('/').to_lowercase();
    REMOVED_SLASH_COMMANDS.contains(&normalized.as_str())
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct InvalidWorkflowCommand(pub String);

pub fn is_reserved_workflow_command(command: &str) -> bool {
    let normalized = command.trim_start_matches('/').to_lowercase();
    if ["ext:", "skill:", "kb:", "ext(", "skill(", "kb("]
        .iter()
        .any(|prefix| normalized.starts_with(prefix))
    {
        return true;
    }
    let reserved = [
        "?",
        "help",
        "exit",
        "quit",
        "t",
        "r",
        "mode",
        "plan",
        "endplan",
        "workflow",
        "extension",
        "builtin",
        "diverge",
        "rename",
        "knowledge",
        "extend",
        "summarize",
        "skill",
        "ext",
        "kb",
    ];
    is_removed_slash_command(&normalized)
        || reserved.contains(&normalized.as_str())
        || crate::agents::execute_commands::list_commands()
            .iter()
            .any(|def| def.name == normalized)
}

fn validate_workflow_command(command: &str) -> Result<()> {
    if command.is_empty()
        || !command.starts_with(|c: char| c.is_ascii_lowercase())
        || !command
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(InvalidWorkflowCommand("Use a single command name starting with a letter, followed by letters, digits, hyphens or underscores.".to_string()).into());
    }
    if is_reserved_workflow_command(command) {
        return Err(InvalidWorkflowCommand(format!(
            "/{command} is reserved by Biorouter. Choose a different workflow command name."
        ))
        .into());
    }
    Ok(())
}

fn save_slash_commands(commands: Vec<SlashCommandMapping>) -> Result<()> {
    Config::global()
        .set_param(SLASH_COMMANDS_CONFIG_KEY, &commands)
        .map_err(|e| anyhow::anyhow!("Failed to save slash commands: {}", e))
}

pub fn remove_commands_for_directory(directory: &std::path::Path) -> Result<usize> {
    let mut commands = configured_commands();
    let before = commands.len();
    commands.retain(|mapping| !PathBuf::from(&mapping.workflow_path).starts_with(directory));
    let removed = before - commands.len();
    save_slash_commands(commands)?;
    Ok(removed)
}

pub fn set_workflow_slash_command(workflow_path: PathBuf, command: Option<String>) -> Result<()> {
    let workflow_path_str = workflow_path.to_string_lossy().to_string();

    let commands = update_workflow_binding(configured_commands(), &workflow_path_str, command)?;
    save_slash_commands(commands)
}

fn update_workflow_binding(
    mut commands: Vec<SlashCommandMapping>,
    workflow_path_str: &str,
    command: Option<String>,
) -> Result<Vec<SlashCommandMapping>> {
    commands.retain(|mapping| mapping.workflow_path != workflow_path_str);

    if let Some(cmd) = command {
        let normalized_cmd = cmd.trim().trim_start_matches('/').to_lowercase();
        validate_workflow_command(&normalized_cmd)?;
        if commands.iter().any(|mapping| {
            mapping
                .command
                .trim_start_matches('/')
                .eq_ignore_ascii_case(&normalized_cmd)
        }) {
            return Err(InvalidWorkflowCommand(format!("/{normalized_cmd} is already assigned to another workflow. Choose a different name or remove its existing binding.")).into());
        }
        commands.push(SlashCommandMapping {
            command: normalized_cmd,
            workflow_path: workflow_path_str.to_string(),
        });
    }

    Ok(commands)
}

pub fn get_workflow_for_command(command: &str) -> Option<PathBuf> {
    resolve_in(configured_commands(), command)
}

/// The resolution itself, free of the config read so it can be tested.
///
/// A name the product RETIRED, or one it reserves for a built-in, must not
/// resolve to a workflow just because a config file still names it.
/// `validate_workflow_command` refuses to create such a mapping, but that only
/// governs mappings made THROUGH it: a file hand-edited, carried over from a
/// version where the name was still free, or written by an agent with a shell,
/// reaches here without ever passing that gate. Resolution is the last place the
/// answer can be no.
///
/// This restores a filter resolution had before the configured/display split:
/// it used to look the command up in the FILTERED catalogue, so a reserved name
/// found nothing. The split correctly gave reads and writes the raw list —
/// resolution is neither, and it silently inherited the raw one.
fn resolve_in(commands: Vec<SlashCommandMapping>, command: &str) -> Option<PathBuf> {
    let normalized = command.trim_start_matches('/').to_lowercase();
    if is_reserved_workflow_command(&normalized) {
        return None;
    }
    commands
        .into_iter()
        .find(|mapping| {
            mapping
                .command
                .trim_start_matches('/')
                .eq_ignore_ascii_case(&normalized)
        })
        .map(|mapping| PathBuf::from(mapping.workflow_path))
}

pub fn resolve_slash_command(command: &str) -> Option<Workflow> {
    let workflow_path = get_workflow_for_command(command)?;

    if !workflow_path.exists() {
        return None;
    }
    let workflow_content = std::fs::read_to_string(&workflow_path).ok()?;
    let workflow = Workflow::from_content(&workflow_content).ok()?;

    Some(workflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    /// Collects formatted tracing output so a test can assert the *level* a
    /// message was emitted at, not merely that it was emitted.
    #[derive(Clone, Default)]
    struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

    impl CapturedLogs {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).to_string()
        }
    }

    impl std::io::Write for CapturedLogs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for CapturedLogs {
        type Writer = CapturedLogs;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn capture<T>(f: impl FnOnce() -> T) -> (T, String) {
        let logs = CapturedLogs::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .finish();
        let value = tracing::subscriber::with_default(subscriber, f);
        (value, logs.text())
    }

    #[test]
    fn an_unset_key_is_the_empty_default_not_a_warning() {
        MISSING_KEY_LOGGED.store(false, Ordering::Relaxed);

        let (commands, logs) = capture(|| {
            let mut all = Vec::new();
            // list_commands runs many times per session; the note must not be
            // repeated once per call.
            for _ in 0..3 {
                all.extend(commands_or_default(Err(ConfigError::NotFound(
                    SLASH_COMMANDS_CONFIG_KEY.to_string(),
                ))));
            }
            all
        });

        assert!(commands.is_empty());
        assert!(
            !logs.contains("WARN"),
            "an absent optional key is the default state; logs were:\n{logs}"
        );
        assert_eq!(
            logs.matches("DEBUG").count(),
            1,
            "the note is worth recording once per process, not once per call; logs were:\n{logs}"
        );
    }

    #[test]
    fn a_real_config_failure_still_warns() {
        let (commands, logs) = capture(|| {
            commands_or_default(Err(ConfigError::DeserializeError(
                "invalid type: string".to_string(),
            )))
        });

        assert!(commands.is_empty());
        assert!(
            logs.contains("WARN") && logs.contains("invalid type: string"),
            "a config that cannot be read is a real problem; logs were:\n{logs}"
        );
    }

    #[test]
    fn configured_commands_are_returned_unchanged() {
        let (commands, logs) = capture(|| {
            commands_or_default(Ok(vec![SlashCommandMapping {
                command: "review".to_string(),
                workflow_path: "/tmp/review.yaml".to_string(),
            }]))
        });

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].command, "review");
        assert!(logs.is_empty(), "logs were:\n{logs}");
    }
    #[test]
    fn workflow_names_cannot_shadow_commands_or_resource_markers() {
        for command in [
            "compact",
            "effort",
            "diverge",
            "knowledge",
            "rename",
            "extend",
            "extension",
            "skill",
            "ext",
            "kb",
            "prompt",
            "two words",
            "ext:developer",
            "",
            "../file",
        ] {
            assert!(validate_workflow_command(command).is_err(), "{command}");
        }
        assert!(validate_workflow_command("review-paper_2").is_ok());
    }
    #[test]
    fn duplicate_bindings_fail_without_dropping_unrelated_legacy_entries() {
        let existing = vec![
            SlashCommandMapping {
                command: "compact".into(),
                workflow_path: "/old.yaml".into(),
            },
            SlashCommandMapping {
                command: "review".into(),
                workflow_path: "/review.yaml".into(),
            },
        ];
        let error = update_workflow_binding(existing.clone(), "/new.yaml", Some("review".into()))
            .unwrap_err();
        assert!(error.downcast_ref::<InvalidWorkflowCommand>().is_some());
        let updated =
            update_workflow_binding(existing, "/new.yaml", Some("check-report".into())).unwrap();
        assert_eq!(updated.len(), 3);
        assert_eq!(updated[0].command, "compact");
    }
    #[test]
    fn resolution_refuses_a_reserved_or_retired_name_a_config_file_still_holds() {
        // Every entry here is one `validate_workflow_command` would refuse to
        // create, so each can only arrive by a route that skips it: an edited
        // file, an upgrade that reserved the name later, or an agent with a
        // shell writing config.yaml.
        let table = vec![
            SlashCommandMapping {
                command: "help".into(),
                workflow_path: "/hijack.yaml".into(),
            },
            SlashCommandMapping {
                command: "prompt".into(),
                workflow_path: "/retired.yaml".into(),
            },
            SlashCommandMapping {
                command: "/Diverge".into(),
                workflow_path: "/shadow.yaml".into(),
            },
            SlashCommandMapping {
                command: "review-paper".into(),
                workflow_path: "/review.yaml".into(),
            },
        ];
        for refused in ["help", "/help", "prompt", "diverge", "/Diverge"] {
            assert_eq!(
                resolve_in(table.clone(), refused),
                None,
                "/{refused} resolved to a workflow"
            );
        }
        // The negative control. Without it this test passes just as well against
        // a `resolve_in` that returns None for everything.
        assert_eq!(
            resolve_in(table.clone(), "review-paper"),
            Some(PathBuf::from("/review.yaml"))
        );
        assert_eq!(
            resolve_in(table, "/Review-Paper"),
            Some(PathBuf::from("/review.yaml")),
            "the case- and slash-insensitive match must survive the filter"
        );
    }

    #[test]
    fn resource_markers_and_help_alias_are_reserved_but_legacy_names_are_not() {
        for command in [
            "ext:computercontroller",
            "skill:rna",
            "kb:research",
            "ext(developer)",
            "skill(rna)",
            "kb(research)",
            "?",
            "/EFFORT",
            "/ext:developer",
        ] {
            assert!(is_reserved_workflow_command(command), "{command}");
        }
        let names = [
            "123",
            "foo.bar",
            "révision",
            "/ext:foo",
            "skill(foo)",
            "?",
            "/EFFORT",
            "FOO.BAR",
        ];
        let catalog = command_catalog(
            names
                .into_iter()
                .map(|command| SlashCommandMapping {
                    command: command.to_string(),
                    workflow_path: "/legacy.yaml".into(),
                })
                .collect(),
        );
        assert_eq!(
            catalog
                .iter()
                .map(|entry| entry.command.as_str())
                .collect::<Vec<_>>(),
            ["123", "foo.bar", "révision"]
        );
    }
}
