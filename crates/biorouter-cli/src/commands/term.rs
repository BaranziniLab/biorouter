use anyhow::{anyhow, Result};
use biorouter::conversation::message::{Message, MessageContent, MessageMetadata};
use biorouter::session::session_manager::{ConversationRevision, TruncateOutcome};
use biorouter::session::{SessionManager, SessionType};
use chrono;
use rmcp::model::Role;

use crate::session::{build_session, SessionBuilderConfig};

use clap::ValueEnum;

#[derive(ValueEnum, Clone, Debug)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    #[value(alias = "pwsh")]
    Powershell,
}

struct ShellConfig {
    script_template: &'static str,
    command_not_found: Option<&'static str>,
}

impl Shell {
    fn config(&self) -> &'static ShellConfig {
        match self {
            Shell::Bash => &BASH_CONFIG,
            Shell::Zsh => &ZSH_CONFIG,
            Shell::Fish => &FISH_CONFIG,
            Shell::Powershell => &POWERSHELL_CONFIG,
        }
    }
}

static BASH_CONFIG: ShellConfig = ShellConfig {
    script_template: r#"export BIOROUTER_SESSION_ID="{session_id}"
alias @biorouter='{biorouter_bin} term run'
alias @g='{biorouter_bin} term run'

biorouter_preexec() {
    [[ "$1" =~ ^biorouter\ term ]] && return
    [[ "$1" =~ ^(@biorouter|@g)($|[[:space:]]) ]] && return
    ('{biorouter_bin}' term log "$1" &) 2>/dev/null
}

if [[ -z "$biorouter_preexec_installed" ]]; then
    biorouter_preexec_installed=1
    trap 'biorouter_preexec "$BASH_COMMAND"' DEBUG
fi{command_not_found_handler}"#,
    command_not_found: Some(
        r#"

command_not_found_handle() {
    echo "🪿 Command '$1' not found. Asking biorouter..."
    '{biorouter_bin}' term run "$@"
    return 0
}"#,
    ),
};

static ZSH_CONFIG: ShellConfig = ShellConfig {
    script_template: r#"export BIOROUTER_SESSION_ID="{session_id}"
alias @biorouter='{biorouter_bin} term run'
alias @g='{biorouter_bin} term run'

biorouter_preexec() {
    [[ "$1" =~ ^biorouter\ term ]] && return
    [[ "$1" =~ ^(@biorouter|@g)($|[[:space:]]) ]] && return
    ('{biorouter_bin}' term log "$1" &) 2>/dev/null
}

autoload -Uz add-zsh-hook
add-zsh-hook preexec biorouter_preexec{command_not_found_handler}"#,
    command_not_found: Some(
        r#"

command_not_found_handler() {
    echo "🪿 Command '$1' not found. Asking biorouter..."
    '{biorouter_bin}' term run "$@"
    return 0
}"#,
    ),
};

static FISH_CONFIG: ShellConfig = ShellConfig {
    script_template: r#"set -gx BIOROUTER_SESSION_ID "{session_id}"
function @biorouter; {biorouter_bin} term run $argv; end
function @g; {biorouter_bin} term run $argv; end

function biorouter_preexec --on-event fish_preexec
    string match -q -r '^biorouter term' -- $argv[1]; and return
    string match -q -r '^(@biorouter|@g)($|\s)' -- $argv[1]; and return
    {biorouter_bin} term log "$argv[1]" 2>/dev/null &
end"#,
    command_not_found: None,
};

static POWERSHELL_CONFIG: ShellConfig = ShellConfig {
    script_template: r#"$env:BIOROUTER_SESSION_ID = "{session_id}"
function @biorouter {{ & '{biorouter_bin}' term run @args }}
function @g {{ & '{biorouter_bin}' term run @args }}

Set-PSReadLineKeyHandler -Chord Enter -ScriptBlock {{
    $line = $null
    [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$null)
    if ($line -notmatch '^biorouter term' -and $line -notmatch '^(@biorouter|@g)($|\s)') {{
        Start-Job -ScriptBlock {{ & '{biorouter_bin}' term log $using:line }} | Out-Null
    }}
    [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
}}"#,
    command_not_found: None,
};

pub async fn handle_term_init(
    shell: Shell,
    name: Option<String>,
    with_command_not_found: bool,
) -> Result<()> {
    let config = shell.config();
    let session_manager = SessionManager::instance();

    let working_dir = std::env::current_dir()?;
    let named_session = if let Some(ref name) = name {
        let sessions = session_manager
            .list_sessions_by_types(&[SessionType::Terminal])
            .await?;
        sessions.into_iter().find(|s| s.name == *name)
    } else {
        None
    };

    let session = match named_session {
        Some(s) => s,
        None => {
            // Same precondition every other row-creating path checks, and for the
            // same reason (PR #183): `build_session` is where a missing provider
            // is discovered, it runs long after this, and by then the row is in
            // the store. `term init` was the one command that skipped it, so a
            // fresh install collected an orphan "Biorouter Term Session" for
            // every shell that sourced the script. The guard resolves the
            // configured default itself, which is why no flags are passed —
            // `term init` accepts none.
            crate::cli::refuse_unconfigured_before_creating_a_row(None, None)?;
            let session = session_manager
                .create_session(
                    working_dir,
                    "Biorouter Term Session".to_string(),
                    SessionType::Terminal,
                )
                .await?;

            if let Some(name) = name {
                session_manager
                    .update(&session.id)
                    .user_provided_name(name)
                    .apply()
                    .await?;
            }

            session
        }
    };

    let biorouter_bin = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "biorouter".to_string());

    let command_not_found_handler = if with_command_not_found {
        config
            .command_not_found
            .map(|s| s.replace("{biorouter_bin}", &biorouter_bin))
            .unwrap_or_default()
    } else {
        String::new()
    };

    let script = config
        .script_template
        .replace("{session_id}", &session.id)
        .replace("{biorouter_bin}", &biorouter_bin)
        .replace("{command_not_found_handler}", &command_not_found_handler);

    println!("{}", script);
    Ok(())
}

pub async fn handle_term_log(command: String) -> Result<()> {
    let session_id = std::env::var("BIOROUTER_SESSION_ID").map_err(|_| {
        anyhow!(
            "BIOROUTER_SESSION_ID not set. Run 'eval \"$(biorouter term init <shell>)\"' first."
        )
    })?;

    let message = Message::new(
        Role::User,
        chrono::Utc::now().timestamp_millis(),
        vec![MessageContent::text(command)],
    )
    .with_metadata(MessageMetadata::user_only());

    let session_manager = SessionManager::instance();
    session_manager.add_message(&session_id, &message).await?;

    Ok(())
}

/// The shell commands `term log` recorded since the last assistant reply, and
/// the revision of the stored conversation that reading of them is based on.
///
/// `term run` shows those commands to the model as `<shell_history>` and drops
/// them from the stored conversation, so they arrive once, in context, instead
/// of twice. Deciding what to drop and dropping it are separate steps because
/// the two happen at different times against a database other processes write
/// to: `basis` is what lets the drop be bounded to the rows this view actually
/// covered.
struct ShellHistoryPlan {
    /// The recorded commands, oldest first.
    commands: Vec<String>,
    /// `created` of the oldest recorded command — the cut point. `None` when
    /// there is nothing to fold.
    cut_from: Option<i64>,
    /// The revision the whole plan was read from.
    basis: ConversationRevision,
}

impl ShellHistoryPlan {
    /// Prefix `prompt` with the recorded commands, if there are any.
    fn compose(&self, prompt: String) -> String {
        if self.commands.is_empty() {
            return prompt;
        }
        format!(
            "<shell_history>\n{}\n</shell_history>\n\n{}",
            self.commands.join("\n"),
            prompt
        )
    }
}

/// Read the trailing run of non-assistant messages — what `term log` recorded
/// since the last reply — together with the revision that view is based on.
async fn plan_shell_history(
    session_manager: &SessionManager,
    session_id: &str,
) -> Result<ShellHistoryPlan> {
    let (session, basis) = session_manager.snapshot_for_rewrite(session_id).await?;

    // Read the revision BEFORE the conversation (`snapshot_for_rewrite`'s
    // ordering) and then keep only the prefix that revision describes. A
    // `term log` landing between the two reads is therefore neither folded
    // into the prompt nor dropped from the store: it stays a plain message the
    // resumed session shows the model on its own. The other order would leave
    // it unseen here and below the watermark there — deleted, and never shown.
    let seen: &[Message] = session
        .conversation
        .as_ref()
        .map(|conv| {
            let msgs = conv.messages();
            &msgs[..basis.message_count().min(msgs.len())]
        })
        .unwrap_or(&[]);

    let trailing: Vec<&Message> = seen
        .iter()
        .rev()
        .take_while(|m| m.role != Role::Assistant)
        .collect();

    Ok(ShellHistoryPlan {
        cut_from: trailing.last().map(|oldest| oldest.created),
        commands: trailing
            .iter()
            .rev() // back to chronological order
            .map(|m| m.as_concat_text())
            .collect(),
        basis,
    })
}

/// Drop the messages `plan` folded into the prompt.
///
/// Bounded by the plan's own basis: anything appended after that view — a
/// `term log` from another pane, a GUI turn on the same session — is outside
/// the tail this command asked to drop and is left alone, even though its
/// `created` of "now" puts it inside the open-ended timestamp range.
async fn apply_shell_history_plan(
    session_manager: &SessionManager,
    session_id: &str,
    plan: &ShellHistoryPlan,
) -> Result<TruncateOutcome> {
    let Some(cut_from) = plan.cut_from else {
        return Ok(TruncateOutcome::Truncated { removed: 0 });
    };
    session_manager
        .truncate_conversation_bounded(session_id, cut_from, plan.basis)
        .await
}

pub async fn handle_term_run(prompt: Vec<String>) -> Result<()> {
    let prompt = prompt.join(" ");
    let session_id = std::env::var("BIOROUTER_SESSION_ID").map_err(|_| {
        anyhow!(
            "BIOROUTER_SESSION_ID not set.\n\n\
             Add to your shell config (~/.zshrc or ~/.bashrc):\n    \
             eval \"$(biorouter term init zsh)\"\n\n\
             Then restart your terminal or run: source ~/.zshrc"
        )
    })?;

    let working_dir = std::env::current_dir()?;
    let session_manager = SessionManager::instance();

    // Shell-following: `term run` intentionally tracks the user's shell cwd
    // mid-conversation, so it is the ONE sanctioned caller of the unguarded
    // working-dir update (#44). Everything else goes through the empty-chat-only
    // guarded update.
    session_manager
        .force_update_working_dir_unguarded(&session_id, working_dir)
        .await?;

    let plan = plan_shell_history(&session_manager, &session_id).await?;
    // A refused cut is not a reason to refuse the user's prompt: it means the
    // session row this plan was read from is gone, so the commands it wanted to
    // drop went with it. Nothing was deleted, nothing is duplicated, and the
    // prompt below still carries them. Say so and carry on.
    match apply_shell_history_plan(&session_manager, &session_id, &plan).await? {
        TruncateOutcome::Truncated { .. } => {}
        outcome => tracing::warn!(
            ?outcome,
            session_id,
            "shell history not folded out of the stored conversation"
        ),
    }
    let prompt_with_context = plan.compose(prompt);

    let config = SessionBuilderConfig {
        session_id: Some(session_id),
        resume: true,
        interactive: false,
        quiet: true,
        ..Default::default()
    };

    let mut session = build_session(config).await;
    session.headless(prompt_with_context).await?;

    Ok(())
}

/// Handle `biorouter term info` - print compact session info for prompt integration
pub async fn handle_term_info() -> Result<()> {
    let session_id = match std::env::var("BIOROUTER_SESSION_ID") {
        Ok(id) => id,
        Err(_) => return Ok(()),
    };

    let session_manager = SessionManager::instance();
    let session = session_manager.get_session(&session_id, false).await.ok();
    let total_tokens = session.as_ref().and_then(|s| s.total_tokens).unwrap_or(0) as usize;

    let config = biorouter::config::Config::global();
    let model_name = config
        .get_biorouter_model()
        .ok()
        .map(|name| {
            let short = name.rsplit('/').next().unwrap_or(&name);
            if let Some(stripped) = short.strip_prefix("biorouter-") {
                stripped.to_string()
            } else {
                short.to_string()
            }
        })
        .unwrap_or_else(|| "?".to_string());

    let context_limit = config
        .get_biorouter_model()
        .ok()
        .and_then(|model_name| biorouter::model::ModelConfig::new(&model_name).ok())
        .map(|mc| mc.context_limit())
        .unwrap_or(128_000);

    let percentage = if context_limit > 0 {
        ((total_tokens as f64 / context_limit as f64) * 100.0).round() as usize
    } else {
        0
    };

    let filled = (percentage / 20).min(5);
    let empty = 5 - filled;
    let dots = format!("{}{}", "●".repeat(filled), "○".repeat(empty));

    println!("{} {}", dots, model_name);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn term_session(sm: &SessionManager) -> String {
        sm.create_session(
            std::env::temp_dir(),
            "Biorouter Term Session".to_string(),
            SessionType::Terminal,
        )
        .await
        .unwrap()
        .id
    }

    /// A shell command as `term log` writes it.
    fn logged(created: i64, text: &str) -> Message {
        Message::new(
            Role::User,
            created,
            vec![MessageContent::text(text.to_string())],
        )
        .with_metadata(MessageMetadata::user_only())
    }

    fn replied(created: i64, text: &str) -> Message {
        Message::new(
            Role::Assistant,
            created,
            vec![MessageContent::text(text.to_string())],
        )
    }

    async fn stored_texts(sm: &SessionManager, id: &str) -> Vec<String> {
        sm.get_session(id, true)
            .await
            .unwrap()
            .conversation
            .map(|c| c.messages().iter().map(|m| m.as_concat_text()).collect())
            .unwrap_or_default()
    }

    /// #51 NF-C: `term run` folds the commands `term log` recorded since the
    /// last reply into its prompt and then deletes them. The delete is ranged
    /// on `created_timestamp >= ?`, so a `term log` from another pane — or a
    /// GUI turn on the same session — that lands while this process is still
    /// building its prompt has a `created` of "now" and is inside the range by
    /// construction. It is destroyed, silently, after its writer was already
    /// told the append succeeded, and it is not in the prompt either.
    ///
    /// Bounding the cut to the revision the plan was read from is the fix. A
    /// re-read of the revision inside the cut would NOT be: the append is
    /// already committed by then, so a fresh watermark covers it.
    #[tokio::test]
    async fn a_concurrent_term_log_is_not_swept_into_the_cut() {
        let temp = TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let id = term_session(&sm).await;

        sm.add_message(&id, &logged(1, "ls -la")).await.unwrap();
        sm.add_message(&id, &replied(2, "here is the listing"))
            .await
            .unwrap();
        sm.add_message(&id, &logged(3, "cargo build"))
            .await
            .unwrap();

        // `term run` reads the history and decides what to fold in.
        let plan = plan_shell_history(&sm, &id).await.unwrap();
        assert_eq!(plan.commands, vec!["cargo build".to_string()], "seeded");

        // Another writer appends while this process is still working.
        sm.add_message(&id, &logged(9, "git push")).await.unwrap();

        apply_shell_history_plan(&sm, &id, &plan).await.unwrap();

        assert_eq!(
            stored_texts(&sm, &id).await,
            vec![
                "ls -la".to_string(),
                "here is the listing".to_string(),
                "git push".to_string(),
            ],
            "the folded command goes; the append this process never saw stays"
        );
    }

    /// The two halves must agree: everything the cut removes has to come back
    /// as `<shell_history>`, or `term run` has silently eaten the user's own
    /// commands instead of re-presenting them.
    #[tokio::test]
    async fn everything_the_cut_removes_comes_back_in_the_prompt() {
        let temp = TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let id = term_session(&sm).await;

        sm.add_message(&id, &logged(1, "cd /tmp")).await.unwrap();
        sm.add_message(&id, &replied(2, "ok")).await.unwrap();
        sm.add_message(&id, &logged(3, "make")).await.unwrap();
        sm.add_message(&id, &logged(4, "make test")).await.unwrap();

        let plan = plan_shell_history(&sm, &id).await.unwrap();
        apply_shell_history_plan(&sm, &id, &plan).await.unwrap();

        let survivors = stored_texts(&sm, &id).await;
        assert_eq!(survivors, vec!["cd /tmp".to_string(), "ok".to_string()]);

        let composed = plan.compose("why did that fail?".to_string());
        assert_eq!(
            composed, "<shell_history>\nmake\nmake test\n</shell_history>\n\nwhy did that fail?",
            "the dropped commands, oldest first, ahead of the user's question"
        );
    }

    /// Nothing recorded since the last reply: no cut, no wrapper.
    #[tokio::test]
    async fn no_recorded_commands_leaves_the_conversation_alone() {
        let temp = TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let id = term_session(&sm).await;

        sm.add_message(&id, &logged(1, "ls")).await.unwrap();
        sm.add_message(&id, &replied(2, "listing")).await.unwrap();

        let plan = plan_shell_history(&sm, &id).await.unwrap();
        assert!(plan.cut_from.is_none());
        assert_eq!(
            apply_shell_history_plan(&sm, &id, &plan).await.unwrap(),
            TruncateOutcome::Truncated { removed: 0 }
        );

        assert_eq!(
            stored_texts(&sm, &id).await,
            vec!["ls".to_string(), "listing".to_string()]
        );
        assert_eq!(plan.compose("hello".to_string()), "hello");
    }

    /// The session id was recycled underneath us, so the plan's watermark
    /// describes rowids from a conversation that no longer exists. The cut is
    /// refused outright rather than applied to whatever now holds the id.
    #[tokio::test]
    async fn a_recycled_session_id_refuses_the_cut() {
        let temp = TempDir::new().unwrap();
        let sm = SessionManager::new(temp.path().to_path_buf());
        let id = term_session(&sm).await;

        sm.add_message(&id, &logged(1, "first incarnation"))
            .await
            .unwrap();
        let plan = plan_shell_history(&sm, &id).await.unwrap();
        assert_eq!(plan.commands, vec!["first incarnation".to_string()]);

        sm.clear_all_sessions().await.unwrap();
        // The allocator no longer restarts once the table is empty — `N` comes
        // from `session_id_high_water`, which a reset does not lower. The cut's
        // refusal still has to hold when an id IS recycled, because a build
        // without the mark sharing the file, or a restored backup, can do it;
        // the seam reproduces exactly that state.
        sm.forget_minted_session_ids_for_test().await.unwrap();
        let recycled = term_session(&sm).await;
        assert_eq!(
            recycled, id,
            "the fixture must reproduce the id reuse, or this test proves nothing"
        );
        sm.add_message(&id, &logged(2, "second incarnation"))
            .await
            .unwrap();

        assert_eq!(
            apply_shell_history_plan(&sm, &id, &plan).await.unwrap(),
            TruncateOutcome::Stale
        );
        assert_eq!(
            stored_texts(&sm, &id).await,
            vec!["second incarnation".to_string()],
            "a refused cut must not delete anything"
        );
    }

    /// `term init` creates a session row, and was the one command that did so
    /// without the provider precondition.
    ///
    /// PR #183 put `refuse_unconfigured_before_creating_a_row` above all four
    /// row-creating paths in `cli.rs` and pinned them with a source guard scoped
    /// to `get_or_create_session_id` — which cannot see this module. So on a
    /// fresh install every shell that sourced the `term init` script left an
    /// orphan "Biorouter Term Session" behind, discovered only when
    /// `build_session` ran much later and found no provider.
    ///
    /// Structural, in the style of that guard, and it has to be: the refusal arm
    /// is `std::process::exit(1)`, which no unit test survives, and the success
    /// arm reads the process-global `Config`. What can be asserted is the thing
    /// that was wrong — the ORDER. The check is above the write.
    #[test]
    fn term_init_refuses_an_unconfigured_run_before_it_creates_a_row() {
        let src = include_str!("term.rs");
        let (production, _) = src
            .split_once("\n#[cfg(test)]")
            .expect("term.rs has no test module");
        let (_, after) = production
            .split_once("pub async fn handle_term_init(")
            .expect("handle_term_init is gone");
        let (body, _) = after
            .split_once("\npub async fn ")
            .expect("could not find the end of handle_term_init");

        let guard = body
            .find("refuse_unconfigured_before_creating_a_row(")
            .expect("term init creates a session row without the provider precondition");
        let create = body
            .find(".create_session(")
            .expect("handle_term_init no longer creates a row");
        assert!(
            guard < create,
            "the precondition must be checked BEFORE the row is written, or a fresh \
             install leaves an orphan session behind for every shell that sourced the script"
        );
        assert_eq!(
            production.matches(".create_session(").count(),
            1,
            "another row-creating path appeared in term.rs; give it its own precondition \
             check, then update this count"
        );
    }
}
