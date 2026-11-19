/// The terminal format sentence that opens [`get_cli_prompt`]. A test pins the
/// two together, so the non-interactive text arm below can reuse it without
/// the interactive prompt changing by a byte.
const TERMINAL_FORMAT: &str = "You are being accessed through a command-line interface. Your output is shown in a terminal in a monospace
font, so keep responses short and skip decorative formatting; a one-line answer is good when it suffices.";

/// What a non-interactive session is told about itself. Each sentence matches
/// what the CLI really does with such a session: `Session::headless` sends one
/// message, approvals auto-deny and elicitations auto-cancel, and a reply with
/// no tool call ends the turn (the agent loop's `exit_chat`). It says "your
/// turn ends", not "the run ends at the first reply", because a reply cut at
/// the length limit, a workflow's final output tool and the opt-in done gate
/// can each send the model back to work.
const HEADLESS_PROMPT: &str =
    "This run is non-interactive: nobody can answer questions or approve actions. \
Your turn ends when you reply without a tool call, so keep working until the task is complete, \
and work around a failed step instead of stopping. \
Before your final reply, check with a tool call that each requested file exists and is not empty. \
Never claim a file was written unless a command wrote it. \
Brevity applies to chat replies, not to files you are asked to produce.";

/// Returns a system prompt extension that explains CLI-specific functionality
pub fn get_cli_prompt() -> String {
    let newline_key = super::input::get_newline_key().to_ascii_uppercase();
    format!(
        "You are being accessed through a command-line interface. Your output is shown in a terminal in a monospace
font, so keep responses short and skip decorative formatting; a one-line answer is good when it suffices.

The following slash commands are available
- you can let the user know about them if they need help:

- /exit or /quit - Exit the chat
- /t - Toggle Light/Dark/Ansi themes in the classic CLI only (BIOROUTER_CLI_CLASSIC=1)
- /? or /help - Display help message

Additional keyboard shortcuts:
- Ctrl+C - Interrupt the current interaction (resets to before the interrupted request)
- Ctrl+{newline_key} - Add a newline
- Tab - Complete slash commands and /skill: or /ext: references
- Up/Down arrows - Navigate command history"
    )
}

/// The block for a session nobody is typing into (`biorouter run` without
/// `--interactive`, `biorouter term run`, `biorouter bench`).
pub fn get_headless_prompt() -> String {
    HEADLESS_PROMPT.to_string()
}

/// The CLI's system prompt extension for one session, chosen from how the
/// session is run. `build_session` appends it as an extra, so it reaches the
/// model under `BIOROUTER_SYSTEM_PROMPT_FILE_PATH` too: an override replaces
/// only the base prompt.
///
/// - Interactive: [`get_cli_prompt`] unchanged. Only `Session::interactive`
///   honours its slash commands and keyboard shortcuts.
/// - Not interactive: the headless block. The terminal format sentence comes
///   first only for `text` output, which a person reads in a shell (`term
///   run`, `run --text` at a terminal); a program reads `json` and
///   `stream-json`, so "keep responses short" has no reader there.
pub fn session_prompt_extension(interactive: bool, output_format: &str) -> String {
    if interactive {
        get_cli_prompt()
    } else if output_format == "text" {
        format!("{TERMINAL_FORMAT}\n\n{HEADLESS_PROMPT}")
    } else {
        get_headless_prompt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter::agents::PromptManager;

    /// The CLI prompt must carry the terminal-specific terseness guidance so
    /// CLI responses stay short (the field-wide convention for terminal agents).
    #[test]
    fn cli_prompt_has_terseness_and_slash_commands() {
        let p = get_cli_prompt();
        assert!(
            p.contains("monospace") && p.contains("keep responses short"),
            "CLI prompt must include terminal terseness guidance"
        );
        assert!(
            p.contains("/exit"),
            "CLI prompt must still list slash commands"
        );
    }

    /// The non-interactive text arm reuses the interactive prompt's opening
    /// sentence; this keeps the copy and the original from drifting apart.
    #[test]
    fn terminal_format_sentence_is_the_cli_prompts_opening() {
        assert!(get_cli_prompt().starts_with(TERMINAL_FORMAT));
    }

    #[test]
    fn an_interactive_session_keeps_the_cli_prompt_unchanged() {
        for format in ["text", "json", "stream-json"] {
            let p = session_prompt_extension(true, format);
            assert_eq!(p, get_cli_prompt(), "interactive, {format}");
            assert!(p.contains("/exit") && p.contains("keep responses short"));
            assert!(!p.contains(HEADLESS_PROMPT), "interactive, {format}");
        }
    }

    /// The BioRouterBench case: `biorouter run --output-format stream-json
    /// --instructions FILE`. A program reads the output, so neither the
    /// terminal sentence nor the slash commands apply.
    #[test]
    fn a_stream_json_run_gets_only_the_headless_block() {
        for format in ["stream-json", "json"] {
            let p = session_prompt_extension(false, format);
            assert_eq!(p, HEADLESS_PROMPT, "{format}");
            for absent in ["slash commands", "one-line answer", "Ctrl+C", "monospace"] {
                assert!(!p.contains(absent), "{format} run carries {absent:?}");
            }
        }
    }

    /// `biorouter term run`: non-interactive, `text` output, read by a person
    /// in their shell. It keeps the format sentence, drops the slash commands
    /// and adds the headless block.
    #[test]
    fn a_text_run_keeps_the_format_sentence_and_adds_the_headless_block() {
        let p = session_prompt_extension(false, "text");
        assert!(p.starts_with(TERMINAL_FORMAT));
        assert!(p.ends_with(HEADLESS_PROMPT));
        for absent in ["slash commands", "/exit", "Ctrl+C", "Up/Down arrows"] {
            assert!(!p.contains(absent), "text run carries {absent:?}");
        }
    }

    /// Every behaviour the headless block names, so a rewording cannot drop
    /// one silently.
    #[test]
    fn the_headless_block_says_what_a_headless_run_does() {
        let p = get_headless_prompt();
        for needed in [
            "non-interactive",
            "nobody can answer questions or approve actions",
            "Your turn ends when you reply without a tool call",
            "keep working until the task is complete",
            "work around a failed step",
            "with a tool call that each requested file exists and is not empty",
            "Never claim a file was written unless a command wrote it",
            "Brevity applies to chat replies, not to files you are asked to produce",
        ] {
            assert!(p.contains(needed), "headless block lacks {needed:?}");
        }
    }

    /// About 90 tokens: small next to the interactive block it replaces.
    #[tokio::test]
    async fn the_headless_block_is_about_ninety_tokens() {
        let counter = biorouter::token_counter::create_token_counter()
            .await
            .expect("the bundled tokenizer loads");
        let headless = counter.count_tokens(HEADLESS_PROMPT);
        let cli = counter.count_tokens(&get_cli_prompt());
        println!("headless block {headless} tokens, interactive CLI block {cli} tokens");
        assert!(
            (75..=100).contains(&headless),
            "headless block is {headless} tokens"
        );
    }

    /// The benchmark replaced the system prompt with
    /// `BIOROUTER_SYSTEM_PROMPT_FILE_PATH`. `build_session` calls
    /// `Agent::extend_system_prompt` (→ `add_system_prompt_extra`) with this
    /// extension and `Agent::override_system_prompt` (→
    /// `set_system_prompt_override`) with the file; the override replaces only
    /// the base prompt and every extra is still appended, so the headless block
    /// reaches the model under the override. Built here through the same
    /// `PromptManager` the agent renders each turn with.
    #[test]
    fn the_headless_block_survives_a_system_prompt_override() {
        const OVERRIDE: &str = "STUDY PROMPT: answer in /app/answer.txt.";
        let render = |interactive: bool, format: &str| {
            let mut manager = PromptManager::new();
            manager.add_system_prompt_extra(session_prompt_extension(interactive, format));
            manager.set_system_prompt_override(OVERRIDE.to_string());
            manager.builder().build()
        };

        let headless = render(false, "stream-json");
        assert!(headless.starts_with(OVERRIDE), "{headless}");
        assert!(
            headless.contains(&format!("# Additional Instructions:\n\n{HEADLESS_PROMPT}")),
            "{headless}"
        );
        assert!(!headless.contains("slash commands"), "{headless}");
        assert!(!headless.contains("one-line answer"), "{headless}");

        let interactive = render(true, "text");
        assert!(interactive.starts_with(OVERRIDE), "{interactive}");
        assert!(!interactive.contains(HEADLESS_PROMPT), "{interactive}");
        assert!(interactive.contains("/exit"), "{interactive}");
        assert!(
            interactive.contains("keep responses short"),
            "{interactive}"
        );
    }
}
