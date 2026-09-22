use super::completion::BioRouterCompleter;
use anyhow::Result;
use biorouter::config::Config;
use rustyline::Editor;

#[derive(Debug)]
pub enum InputResult {
    Message(String),
    Exit,
    AddExtension(String),
    AddBuiltin(String),
    ToggleTheme,
    SelectTheme(String),
    Retry,
    BioRouterMode(String),
    Plan(PlanCommandOptions),
    EndPlan,
    Clear,
    Workflow(Option<String>),
    Compact,
    ToggleFullToolOutput,
    /// Branch the current conversation into a brand-new session (full history
    /// preserved) and open it in a fresh Biorouter desktop window. An optional
    /// name is given to the new branch.
    Diverge(Option<String>),
    /// Rename the current session.
    Rename(String),
}

#[derive(Debug)]
pub struct PlanCommandOptions {
    pub message_text: String,
}

struct CtrlCHandler;

impl rustyline::ConditionalEventHandler for CtrlCHandler {
    /// Handle Ctrl+C to clear the line if text is entered, otherwise exit the session.
    fn handle(
        &self,
        _event: &rustyline::Event,
        _n: usize,
        _positive: bool,
        ctx: &rustyline::EventContext,
    ) -> Option<rustyline::Cmd> {
        if !ctx.line().is_empty() {
            Some(rustyline::Cmd::Kill(rustyline::Movement::WholeBuffer))
        } else {
            Some(rustyline::Cmd::Interrupt)
        }
    }
}

pub fn get_newline_key() -> char {
    Config::global()
        .get_param::<String>("BIOROUTER_CLI_NEWLINE_KEY")
        .ok()
        .and_then(|s| s.chars().next())
        .map(|c| c.to_ascii_lowercase())
        .unwrap_or('j')
}

pub fn get_input(
    editor: &mut Editor<BioRouterCompleter, rustyline::history::DefaultHistory>,
) -> Result<InputResult> {
    let newline_key = get_newline_key();
    editor.bind_sequence(
        rustyline::KeyEvent(
            rustyline::KeyCode::Char(newline_key),
            rustyline::Modifiers::CTRL,
        ),
        rustyline::EventHandler::Simple(rustyline::Cmd::Newline),
    );

    editor.bind_sequence(
        rustyline::KeyEvent(rustyline::KeyCode::Char('c'), rustyline::Modifiers::CTRL),
        rustyline::EventHandler::Conditional(Box::new(CtrlCHandler)),
    );

    let prompt = get_input_prompt_string();

    let input = match editor.readline(&prompt) {
        Ok(text) => text,
        Err(e) => match e {
            rustyline::error::ReadlineError::Interrupted => return Ok(InputResult::Exit),
            rustyline::error::ReadlineError::Eof => return Ok(InputResult::Exit),
            _ => return Err(e.into()),
        },
    };

    // Add valid input to history (history saving to file is handled in the Session::interactive method)
    if !input.trim().is_empty() {
        editor.add_history_entry(input.as_str())?;
    }

    // Handle non-slash commands first
    if !input.starts_with('/') {
        let trimmed = input.trim();
        if trimmed.is_empty()
            || trimmed.eq_ignore_ascii_case("exit")
            || trimmed.eq_ignore_ascii_case("quit")
        {
            return Ok(if trimmed.is_empty() {
                InputResult::Retry
            } else {
                InputResult::Exit
            });
        }
        return Ok(InputResult::Message(trimmed.to_string()));
    }

    // Handle slash commands
    match handle_slash_command(&input) {
        Some(result) => Ok(result),
        None => Ok(InputResult::Message(input.trim().to_string())),
    }
}

fn handle_slash_command(input: &str) -> Option<InputResult> {
    let input = input.trim();

    // Command prefix constants
    const CMD_EXTENSION: &str = "/extension ";
    const CMD_BUILTIN: &str = "/builtin ";
    const CMD_MODE: &str = "/mode ";
    const CMD_PLAN: &str = "/plan";
    const CMD_ENDPLAN: &str = "/endplan";
    const CMD_CLEAR: &str = "/clear";
    const CMD_WORKFLOW: &str = "/workflow";
    const CMD_COMPACT: &str = "/compact";
    const CMD_SUMMARIZE_DEPRECATED: &str = "/summarize";

    match input {
        "/exit" | "/quit" => Some(InputResult::Exit),
        "/?" | "/help" => {
            print_help();
            Some(InputResult::Retry)
        }
        "/t" => Some(InputResult::ToggleTheme),
        s if s.starts_with("/t ") => {
            let t = s
                .strip_prefix("/t ")
                .unwrap_or_default()
                .trim()
                .to_lowercase();
            if ["light", "dark", "ansi"].contains(&t.as_str()) {
                Some(InputResult::SelectTheme(t))
            } else {
                println!(
                    "Theme Unavailable: {} Available themes are: light, dark, ansi",
                    t
                );
                Some(InputResult::Retry)
            }
        }
        s if s.starts_with(CMD_EXTENSION) => Some(InputResult::AddExtension(
            s.get(CMD_EXTENSION.len()..).unwrap_or("").to_string(),
        )),
        s if s.starts_with(CMD_BUILTIN) => Some(InputResult::AddBuiltin(
            s.get(CMD_BUILTIN.len()..).unwrap_or("").to_string(),
        )),
        s if s.starts_with(CMD_MODE) => Some(InputResult::BioRouterMode(
            s.get(CMD_MODE.len()..).unwrap_or("").to_string(),
        )),
        s if s == CMD_PLAN || s.starts_with("/plan ") => {
            parse_plan_command(s.get(CMD_PLAN.len()..).unwrap_or("").trim().to_string())
        }
        s if s == CMD_ENDPLAN => Some(InputResult::EndPlan),
        s if s == CMD_CLEAR => Some(InputResult::Clear),
        "/diverge" => Some(InputResult::Diverge(None)),
        s if s.starts_with("/diverge ") => {
            let name = s.strip_prefix("/diverge ").unwrap_or("").trim();
            Some(InputResult::Diverge(
                (!name.is_empty()).then(|| name.to_string()),
            ))
        }
        s if s == "/rename" || s.starts_with("/rename ") => {
            let name = s.strip_prefix("/rename").unwrap_or("").trim();
            if name.is_empty() {
                println!("{}", console::style("Usage: /rename <new name>").yellow());
                Some(InputResult::Retry)
            } else {
                Some(InputResult::Rename(name.to_string()))
            }
        }
        s if s == CMD_WORKFLOW || s.starts_with("/workflow ") => parse_workflow_command(s),
        s if s == CMD_COMPACT => Some(InputResult::Compact),
        s if s == CMD_SUMMARIZE_DEPRECATED => {
            println!("{}", console::style("Note: /summarize has been renamed to /compact and will be removed in a future release.").yellow());
            Some(InputResult::Compact)
        }
        "/r" => Some(InputResult::ToggleFullToolOutput),
        _ => None,
    }
}

fn parse_workflow_command(s: &str) -> Option<InputResult> {
    const CMD_WORKFLOW: &str = "/workflow";

    if s == CMD_WORKFLOW {
        // No filepath provided, use default
        return Some(InputResult::Workflow(None));
    }

    // Extract the filepath from the command
    let filepath = s.get(CMD_WORKFLOW.len()..).unwrap_or("").trim();

    if filepath.is_empty() {
        return Some(InputResult::Workflow(None));
    }

    // Validate that the filepath ends with .yaml
    if !filepath.to_lowercase().ends_with(".yaml") {
        println!("{}", console::style("Filepath must end with .yaml").red());
        return Some(InputResult::Retry);
    }

    // Return the filepath for validation in the handler
    Some(InputResult::Workflow(Some(filepath.to_string())))
}

fn parse_plan_command(input: String) -> Option<InputResult> {
    let options = PlanCommandOptions {
        message_text: input.trim().to_string(),
    };

    Some(InputResult::Plan(options))
}

/// Generates the input prompt string for the CLI interface.
fn get_input_prompt_string() -> String {
    // The brand warm tan-brown accent (xterm-256 137 ≈ #af875f), Biorouter's light cream palette
    const ACCENT: console::Color = console::Color::Color256(137);
    if cfg!(target_os = "windows") {
        "Biorouter> ".to_string()
    } else {
        format!(
            "{} {} ",
            console::style("Biorouter").fg(ACCENT).bold(),
            console::style("❯").fg(ACCENT)
        )
    }
}

fn print_help() {
    use console::{style, Color};
    const ACCENT: Color = Color::Color256(137);

    // (command, description) pairs rendered as an aligned two-column table.
    let commands: &[(&str, &str)] = &[
        ("/exit, /quit", "Exit the chat"),
        ("/t", "Toggle Light / Dark / Ansi theme"),
        ("/t <name>", "Set theme directly (light, dark, ansi)"),
        ("/r", "Toggle full (untruncated) tool output"),
        (
            "/extension <cmd>",
            "Add a stdio extension (ENV1=val1 command args…)",
        ),
        (
            "/builtin <names>",
            "Add builtin extensions by name (comma-separated)",
        ),
        (
            "/mode <name>",
            "Set mode: auto, approve, chat, smart_approve",
        ),
        (
            "/plan [message]",
            "Enter plan mode, then optionally act on the plan",
        ),
        ("/endplan", "Exit plan mode, return to normal mode"),
        ("/workflow [file.yaml]", "Save the chat as a workflow"),
        ("/compact", "Compact the chat to reclaim context"),
        ("/clear", "Clear the current chat history"),
        (
            "/diverge [name]",
            "Diverge this chat into a new Biorouter window (keeps full history)",
        ),
        ("/rename <name>", "Rename the current chat"),
        (
            "/goal <condition>",
            "Keep working until the condition is met (/goal clear to stop)",
        ),
        (
            "/loop <interval> <prompt>",
            "Run a prompt on an interval, e.g. /loop 5m … (/loop stop <id>)",
        ),
        (
            "/schedule <spec> <prompt>",
            "Schedule a recurring prompt: 5m, @daily, or a quoted cron",
        ),
        ("/help, /?", "Show this help message"),
    ];

    let width = commands.iter().map(|(c, _)| c.len()).max().unwrap_or(0);

    println!();
    println!("  {} {}", style("▌").fg(ACCENT), style("Commands").bold());
    for (cmd, desc) in commands {
        println!(
            "    {:<width$}   {}",
            style(cmd).fg(ACCENT),
            style(desc).dim(),
            width = width
        );
    }

    let newline_key = get_newline_key().to_ascii_uppercase();
    let nav: &[(&str, &str)] = &[
        ("Ctrl+C", "Clear the current line, or exit when empty"),
        (
            "Ctrl+_KEY_",
            "Insert a newline (set via BIOROUTER_CLI_NEWLINE_KEY)",
        ),
        (
            "Tab",
            "Complete slash commands and routed resource references",
        ),
        ("↑ / ↓", "Navigate command history"),
    ];

    println!();
    println!("  {} {}", style("▌").fg(ACCENT), style("Navigation").bold());
    for (key, desc) in nav {
        let key = key.replace("_KEY_", &newline_key.to_string());
        println!(
            "    {:<width$}   {}",
            style(key).fg(ACCENT),
            style(desc).dim(),
            width = width
        );
    }

    // Pointers to the shell-level management subcommands (run outside a session).
    let shell: &[(&str, &str)] = &[
        (
            "biorouter knowledge",
            "Manage knowledge bases: ingest, lint, query",
        ),
        (
            "biorouter extension",
            "Install extensions from .brxt bundles",
        ),
        ("biorouter skill", "Install skills from .zip files"),
        (
            "biorouter workflow",
            "Install and run workflows (.json / .yaml)",
        ),
        ("biorouter models", "Inspect and set the provider / model"),
        ("biorouter schedule", "Manage scheduled jobs"),
    ];
    println!();
    println!(
        "  {} {} {}",
        style("▌").fg(ACCENT),
        style("Shell commands").bold(),
        style("(run from your terminal)").dim()
    );
    let shell_width = shell.iter().map(|(c, _)| c.len()).max().unwrap_or(0);
    for (cmd, desc) in shell {
        println!(
            "    {:<shell_width$}   {}",
            style(cmd).fg(ACCENT),
            style(desc).dim(),
            shell_width = shell_width
        );
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handle_slash_command() {
        // Test exit commands
        assert!(matches!(
            handle_slash_command("/exit"),
            Some(InputResult::Exit)
        ));
        assert!(matches!(
            handle_slash_command("/quit"),
            Some(InputResult::Exit)
        ));

        // Test help commands
        assert!(matches!(
            handle_slash_command("/help"),
            Some(InputResult::Retry)
        ));
        assert!(matches!(
            handle_slash_command("/?"),
            Some(InputResult::Retry)
        ));

        // Test theme toggle
        assert!(matches!(
            handle_slash_command("/t"),
            Some(InputResult::ToggleTheme)
        ));

        // Test full tool output toggle
        assert!(matches!(
            handle_slash_command("/r"),
            Some(InputResult::ToggleFullToolOutput)
        ));

        // Test extension command
        if let Some(InputResult::AddExtension(cmd)) = handle_slash_command("/extension foo bar") {
            assert_eq!(cmd, "foo bar");
        } else {
            panic!("Expected AddExtension");
        }

        // Test builtin command
        if let Some(InputResult::AddBuiltin(names)) = handle_slash_command("/builtin dev,git") {
            assert_eq!(names, "dev,git");
        } else {
            panic!("Expected AddBuiltin");
        }

        // Test diverge command
        assert!(matches!(
            handle_slash_command("/diverge"),
            Some(InputResult::Diverge(None))
        ));
        assert!(matches!(
            handle_slash_command("  /diverge  "),
            Some(InputResult::Diverge(None))
        ));
        assert!(handle_slash_command("/divergent").is_none());
        // A trailing argument names the new branch.
        if let Some(InputResult::Diverge(name)) = handle_slash_command("/diverge now") {
            assert_eq!(name, Some("now".to_string()));
        } else {
            panic!("Expected Diverge with a name");
        }

        // Test rename command
        if let Some(InputResult::Rename(name)) = handle_slash_command("/rename my chat") {
            assert_eq!(name, "my chat");
        } else {
            panic!("Expected Rename");
        }
        assert!(matches!(
            handle_slash_command("/rename   "),
            Some(InputResult::Retry)
        ));

        // Test unknown commands
        assert!(handle_slash_command("/unknown").is_none());
    }

    #[test]
    fn test_diverge_is_in_completion_registry() {
        // Keep the parser and the autocomplete list in sync.
        assert!(super::super::completion::SLASH_COMMANDS.contains(&"/diverge"));
    }

    // Test whitespace handling
    #[test]
    fn test_whitespace_handling() {
        // Leading/trailing whitespace in extension command
        if let Some(InputResult::AddExtension(cmd)) = handle_slash_command("  /extension foo bar  ")
        {
            assert_eq!(cmd, "foo bar");
        } else {
            panic!("Expected AddExtension");
        }

        // Leading/trailing whitespace in builtin command
        if let Some(InputResult::AddBuiltin(names)) = handle_slash_command("  /builtin dev,git  ") {
            assert_eq!(names, "dev,git");
        } else {
            panic!("Expected AddBuiltin");
        }
    }

    #[test]
    fn test_plan_mode() {
        // Test plan mode with no text
        let result = handle_slash_command("/plan");
        assert!(result.is_some());

        // Test plan mode with text
        let result = handle_slash_command("/plan hello world");
        assert!(result.is_some());
        let options = result.unwrap();
        match options {
            InputResult::Plan(options) => {
                assert_eq!(options.message_text, "hello world");
            }
            _ => panic!("Expected Plan"),
        }
    }

    #[test]
    fn test_workflow_command() {
        // Test workflow with no filepath
        if let Some(InputResult::Workflow(filepath)) = handle_slash_command("/workflow") {
            assert!(filepath.is_none());
        } else {
            panic!("Expected Workflow");
        }

        // Test workflow with filepath
        if let Some(InputResult::Workflow(filepath)) =
            handle_slash_command("/workflow /path/to/file.yaml")
        {
            assert_eq!(filepath, Some("/path/to/file.yaml".to_string()));
        } else {
            panic!("Expected workflow with filepath");
        }

        // Test workflow with invalid extension
        let result = handle_slash_command("/workflow /path/to/file.txt");
        assert!(matches!(result, Some(InputResult::Retry)));
    }

    #[test]
    fn test_get_input_prompt_string() {
        let prompt = get_input_prompt_string();

        // Prompt should always end with a space
        assert!(prompt.ends_with(" "));

        // Prompt should contain the brand label
        assert!(prompt.contains("Biorouter"));

        // On Windows, prompt should be plain text without ANSI codes
        #[cfg(target_os = "windows")]
        {
            assert_eq!(prompt, "Biorouter> ");
            // Ensure no ANSI escape sequences
            assert!(!prompt.contains("\x1b["));
        }

        // On non-Windows, prompt behavior depends on terminal capabilities
        #[cfg(not(target_os = "windows"))]
        {
            // In CI environments, console crate may strip ANSI codes
            let is_ci = std::env::var("CI").is_ok();

            if is_ci {
                // In CI, just verify basic structure - console crate handles ANSI detection
                assert!(prompt.len() >= "Biorouter> ".len());
            } else {
                // In interactive terminals, expect styling to be applied
                // Note: This may still vary based on terminal capabilities
                assert!(prompt.len() >= "Biorouter> ".len());

                // If ANSI codes are present, they should be valid 256-color/bold sequences
                if prompt.contains("\x1b[") {
                    assert!(prompt.contains("38") || prompt.contains("1"));
                }
            }
        }
    }
    #[test]
    fn command_prefixes_do_not_capture_other_slash_names() {
        assert!(handle_slash_command("/planet").is_none());
        assert!(handle_slash_command("/workflow-analysis.yaml").is_none());
    }
}
