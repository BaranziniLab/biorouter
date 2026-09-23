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

#[cfg(test)]
mod tests {
    use super::*;

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
}
