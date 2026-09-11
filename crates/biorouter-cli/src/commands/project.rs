use anyhow::Result;
use chrono::DateTime;
use cliclack::{self, intro, outro};
use std::path::Path;

use crate::commands::needs_terminal;
use crate::project_tracker::ProjectTracker;
use biorouter::utils::safe_truncate;

/// What `biorouter project` and `biorouter projects` say when there is no
/// terminal to prompt on (QA-D F9). `{command}` is the one the user typed.
fn needs_a_terminal(command: &str) -> String {
    format!(
        "`biorouter {command}` is interactive and needs a terminal; from a script, continue a \
         chat with `biorouter run --resume --session-id <id> --text \"<prompt>\"` (`biorouter \
         session list` shows the ids)."
    )
}

/// Format a DateTime for display
fn format_date(date: DateTime<chrono::Utc>) -> String {
    // Format: "2025-05-08 18:15:30"
    date.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Handle the default project command
///
/// Offers options to resume the most recently accessed project
pub fn handle_project_default() -> Result<()> {
    project_default(needs_terminal::prompt_can_run())
}

/// [`handle_project_default`] with the terminal answer supplied, so the refusal
/// is testable without redirecting the test's own stdin.
#[allow(clippy::too_many_lines)]
fn project_default(terminal: bool) -> Result<()> {
    // Before anything else: BOTH paths below need a person — a picker, or (with
    // no projects yet) an interactive `biorouter session`, which under a pipe
    // reads EOF and exits having written an empty session row.
    needs_terminal::require(terminal, &needs_a_terminal("project"))?;
    let tracker = ProjectTracker::load()?;
    let mut projects = tracker.list_projects();

    if projects.is_empty() {
        // If no projects exist, just start a new one in the current directory
        println!("No previous projects found. Starting a new chat in the current directory.");
        let mut command = std::process::Command::new("biorouter");
        command.arg("session");
        let status = command.status()?;

        if !status.success() {
            println!("Failed to run biorouter. Exit code: {:?}", status.code());
        }
        return Ok(());
    }

    // Sort projects by last_accessed (newest first)
    projects.sort_by(|a, b| b.last_accessed.cmp(&a.last_accessed));

    // Get the most recent project
    let project = &projects[0];
    let project_dir = &project.path;

    // Check if the directory exists
    if !Path::new(project_dir).exists() {
        println!(
            "Most recent project directory '{}' no longer exists.",
            project_dir
        );
        return Ok(());
    }

    // Format the path for display
    let path = Path::new(project_dir);
    let components: Vec<_> = path.components().collect();
    let len = components.len();
    let short_path = if len <= 2 {
        project_dir.clone()
    } else {
        let mut path_str = String::new();
        path_str.push_str("...");
        for component in components.iter().skip(len - 2) {
            path_str.push('/');
            path_str.push_str(component.as_os_str().to_string_lossy().as_ref());
        }
        path_str
    };

    // Ask the user what they want to do
    let _ = intro("Biorouter Project Manager");

    let current_dir = std::env::current_dir()?;
    let current_dir_display = current_dir.display();

    let choice = cliclack::select("Choose an option:")
        .item(
            "resume",
            format!("Resume project chat: {}", short_path),
            "Continue with the previous chat",
        )
        .item(
            "fresh",
            format!("Resume project with a fresh chat: {}", short_path),
            "Change to the project directory but start a new chat",
        )
        .item(
            "new",
            format!(
                "Start new project in current directory: {}",
                current_dir_display
            ),
            "Stay in the current directory and start a new chat",
        )
        .interact()?;

    match choice {
        "resume" => {
            let _ = outro(format!("Changing to directory: {}", project_dir));

            // Get the session ID if available
            let session_id = project.last_session_id.clone();

            // Change to the project directory
            std::env::set_current_dir(project_dir)?;

            // Build the command to run biorouter
            let mut command = std::process::Command::new("biorouter");
            command.arg("session");

            if let Some(id) = session_id {
                command.arg("--name").arg(&id).arg("--resume");
                println!("Resuming chat: {}", id);
            }

            // Execute the command
            let status = command.status()?;

            if !status.success() {
                println!("Failed to run biorouter. Exit code: {:?}", status.code());
            }
        }
        "fresh" => {
            let _ = outro(format!(
                "Changing to directory: {} with a fresh chat",
                project_dir
            ));

            // Change to the project directory
            std::env::set_current_dir(project_dir)?;

            // Build the command to run biorouter with a fresh session
            let mut command = std::process::Command::new("biorouter");
            command.arg("session");

            // Execute the command
            let status = command.status()?;

            if !status.success() {
                println!("Failed to run biorouter. Exit code: {:?}", status.code());
            }
        }
        "new" => {
            let _ = outro("Starting a new chat in the current directory");

            // Build the command to run biorouter
            let mut command = std::process::Command::new("biorouter");
            command.arg("session");

            // Execute the command
            let status = command.status()?;

            if !status.success() {
                println!("Failed to run biorouter. Exit code: {:?}", status.code());
            }
        }
        _ => {
            let _ = outro("Operation canceled");
        }
    }

    Ok(())
}

/// Handle the interactive projects command
///
/// Shows a list of projects and lets the user select one to resume
pub fn handle_projects_interactive() -> Result<()> {
    projects_interactive(ProjectTracker::load()?, needs_terminal::prompt_can_run())
}

/// [`handle_projects_interactive`] with the project list and the terminal
/// answer supplied, so the refusal is testable against a list a test owns.
#[allow(clippy::too_many_lines)]
fn projects_interactive(tracker: ProjectTracker, terminal: bool) -> Result<()> {
    let mut projects = tracker.list_projects();

    if projects.is_empty() {
        println!("No projects found.");
        return Ok(());
    }

    // Before the picker. "No projects found" above needs no person, so it
    // still answers under a pipe.
    needs_terminal::require(terminal, &needs_a_terminal("projects"))?;

    // Sort projects by last_accessed (newest first)
    projects.sort_by(|a, b| b.last_accessed.cmp(&a.last_accessed));

    // Format project paths for display
    let project_choices: Vec<(String, String)> = projects
        .iter()
        .enumerate()
        .map(|(i, project)| {
            let path = Path::new(&project.path);
            let components: Vec<_> = path.components().collect();
            let len = components.len();
            let short_path = if len <= 2 {
                project.path.clone()
            } else {
                let mut path_str = String::new();
                path_str.push_str("...");
                for component in components.iter().skip(len - 2) {
                    path_str.push('/');
                    path_str.push_str(component.as_os_str().to_string_lossy().as_ref());
                }
                path_str
            };

            // Include last instruction if available (truncated)
            let instruction_preview =
                project
                    .last_instruction
                    .as_ref()
                    .map_or(String::new(), |instr| {
                        let truncated = safe_truncate(instr, 40);
                        format!(" [{}]", truncated)
                    });

            let formatted_date = format_date(project.last_accessed);
            (
                format!("{}", i + 1), // Value to return
                format!("{} ({}){}", short_path, formatted_date, instruction_preview), // Display text with instruction
            )
        })
        .collect();

    // Let the user select a project
    let _ = intro("Biorouter Project Manager");
    let mut select = cliclack::select("Select a project:");

    // Add each project as an option
    for (value, display) in &project_choices {
        select = select.item(value, display, "");
    }

    // Add a cancel option
    let cancel_value = String::from("cancel");
    select = select.item(&cancel_value, "Cancel", "Don't resume any project");

    let selected = select.interact()?;

    if selected == "cancel" {
        let _ = outro("Project selection canceled.");
        return Ok(());
    }

    // Parse the selected index
    let index = selected.parse::<usize>().unwrap_or(0);
    if index == 0 || index > projects.len() {
        let _ = outro("Invalid selection.");
        return Ok(());
    }

    // Get the selected project
    let project = &projects[index - 1];
    let project_dir = &project.path;

    // Check if the directory exists
    if !Path::new(project_dir).exists() {
        let _ = outro(format!(
            "Project directory '{}' no longer exists.",
            project_dir
        ));
        return Ok(());
    }

    // Ask if the user wants to resume the session or start a new one
    let session_id = project.last_session_id.clone();
    let has_previous_session = session_id.is_some();

    // Change to the project directory first
    std::env::set_current_dir(project_dir)?;
    let _ = outro(format!("Changed to directory: {}", project_dir));

    // Only ask about resuming if there's a previous session
    let resume_session = if has_previous_session {
        let session_choice = cliclack::select("What would you like to do?")
            .item(
                "resume",
                "Resume previous chat",
                "Continue with the previous chat",
            )
            .item(
                "new",
                "Start new chat",
                "Start a fresh chat in this project directory",
            )
            .interact()?;

        session_choice == "resume"
    } else {
        false
    };

    // Build the command to run biorouter
    let mut command = std::process::Command::new("biorouter");
    command.arg("session");

    if resume_session {
        if let Some(id) = session_id {
            command.arg("--name").arg(&id).arg("--resume");
            println!("Resuming chat: {}", id);
        }
    } else {
        println!("Starting new chat");
    }

    // Execute the command
    let status = command.status()?;

    if !status.success() {
        println!("Failed to run biorouter. Exit code: {:?}", status.code());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::needs_terminal::NeedsTerminal;

    /// QA-D F9: `biorouter project` refuses under a pipe with a sentence
    /// naming the scriptable route — and refuses FIRST, before it reads the
    /// project list, because both of its paths need a person.
    #[test]
    fn project_refuses_without_a_terminal_before_it_reads_anything() {
        let err = project_default(false).unwrap_err();
        let refusal = err
            .downcast_ref::<NeedsTerminal>()
            .expect("the typed refusal main maps to exit 2");
        let sentence = refusal.to_string();
        assert!(sentence.contains("`biorouter project`"), "{sentence}");
        assert!(sentence.contains("is interactive"), "{sentence}");
        assert!(
            sentence.contains("biorouter run --resume --session-id"),
            "the refusal must name what to run instead: {sentence}"
        );
    }

    /// `biorouter projects` refuses at its picker, and still answers "No
    /// projects found." without a terminal — that line needs nobody.
    #[test]
    fn projects_refuses_at_the_picker_but_still_reports_an_empty_list() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("projects.json");

        let empty = ProjectTracker::load_from(&file).unwrap();
        assert!(
            projects_interactive(empty, false).is_ok(),
            "an empty list is answered without prompting"
        );

        let project_dir = dir.path().display().to_string();
        std::fs::write(
            &file,
            serde_json::json!({
                "projects": {
                    project_dir.clone(): {
                        "path": project_dir,
                        "last_accessed": "2026-09-10T12:00:00Z",
                        "last_instruction": null,
                        "last_session_id": "20260910_1"
                    }
                }
            })
            .to_string(),
        )
        .unwrap();
        let one = ProjectTracker::load_from(&file).unwrap();
        assert_eq!(one.list_projects().len(), 1, "the fixture must load");

        let err = projects_interactive(one, false).unwrap_err();
        let sentence = err
            .downcast_ref::<NeedsTerminal>()
            .expect("the typed refusal main maps to exit 2")
            .to_string();
        assert!(sentence.contains("`biorouter projects`"), "{sentence}");
    }
}
