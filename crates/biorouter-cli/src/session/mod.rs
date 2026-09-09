mod builder;
mod completion;
mod elicitation;
mod export;
mod input;
pub mod markdown;
pub mod output;
pub mod privacy;
mod prompt;
mod stream_coalesce;
mod task_execution_display;
mod thinking;
mod tui;

use crate::session::task_execution_display::{
    format_task_execution_notification, TASK_EXECUTION_NOTIFICATION_TYPE,
};
use biorouter::conversation::Conversation;

/// The knowledge service the `/workflow` capture reads its selection from.
///
/// A machine with no knowledge store yet is not an error for this purpose — the
/// capture simply has nothing to record — so the caller downgrades a failure to
/// a warning rather than losing the generated workflow over it.
fn knowledge_service_for_enrichment(
) -> anyhow::Result<biorouter_mcp::knowledge::service::KnowledgeService> {
    biorouter_mcp::knowledge::service::KnowledgeService::new_default()
}

use std::io::{IsTerminal, Write};
use std::str::FromStr;
use tokio::signal::ctrl_c;
use tokio_util::task::AbortOnDropHandle;

pub use self::export::message_to_markdown;
use biorouter::agents::turn_abort::{exit as abort_exit, TurnAbortCode, TurnFailed};
use biorouter::agents::AgentEvent;
use biorouter::permission::permission_confirmation::PrincipalType;
use biorouter::permission::Permission;
use biorouter::permission::PermissionConfirmation;
use biorouter::providers::base::Provider;
use biorouter::utils::safe_truncate;
pub use builder::{build_session, unconfigured_precondition, SessionBuilderConfig};
use console::Color;

use anyhow::Result;
use biorouter::agents::extension::{Envs, ExtensionConfig, PLATFORM_EXTENSIONS};
use biorouter::agents::types::RetryConfig;
use biorouter::agents::{Agent, SessionConfig, COMPACT_TRIGGERS};
use biorouter::config::{BioRouterMode, Config};
use completion::BioRouterCompleter;
use input::InputResult;
use rmcp::model::ServerNotification;
use rmcp::model::{ErrorCode, ErrorData};

use biorouter::config::paths::Paths;
use biorouter::conversation::message::{ActionRequiredData, Message, MessageContent};
use rustyline::EditMode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio;
use tokio_util::sync::CancellationToken;
use tracing::warn;

/// Is this assistant message Gate B's turn refusal?
///
/// Keyed on [`biorouter::privacy::refusal::TURN_REFUSAL_MARKER`] — the constant
/// that exists precisely so independent readers can tell a refusal from a
/// completed turn — rather than on a phrase retyped here, which would go quietly
/// false the first time the wording moved.
pub(crate) fn is_privacy_turn_refusal(message: &Message) -> bool {
    message
        .as_concat_text()
        .contains(biorouter::privacy::refusal::TURN_REFUSAL_MARKER)
}

/// Build the `biorouter://diverge` deeplink the CLI hands to the desktop app to
/// open a diverged session in a fresh window. The session id and working dir
/// are URL-encoded so paths with spaces/special characters survive the round
/// trip. Kept as a free function so it can be unit-tested without a session.
pub(crate) fn build_diverge_deeplink(session_id: &str, working_dir: &std::path::Path) -> String {
    let encoded_id = urlencoding::encode(session_id);
    let working_dir_lossy = working_dir.to_string_lossy();
    let encoded_dir = urlencoding::encode(&working_dir_lossy);
    format!("biorouter://diverge?session_id={encoded_id}&dir={encoded_dir}")
}

/// Result of a `/diverge`: the new branched session, the deeplink used to open
/// it, and whether opening the desktop window failed (the branch is persisted
/// regardless).
pub(crate) struct DivergeOutcome {
    pub new_session_id: String,
    pub url: String,
    pub open_error: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct JsonOutput {
    messages: Vec<Message>,
    metadata: JsonMetadata,
}

#[derive(Serialize, Deserialize, Debug)]
struct JsonMetadata {
    total_tokens: Option<i32>,
    /// `"completed"` or `"failed"`. Derived from [`CliSession::last_abort`], not
    /// hardcoded — a turn that never ran must not report success.
    status: String,
    /// The abort code, when `status == "failed"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StreamEvent {
    Message {
        message: Message,
    },
    Notification {
        extension_id: String,
        #[serde(flatten)]
        data: NotificationData,
    },
    ModelChange {
        model: String,
        mode: String,
    },
    Error {
        error: String,
    },
    /// The turn ended without doing its work. Emitted *before* `Complete`, so a
    /// stream-json consumer that only watches for `complete` still terminates,
    /// but one that checks for failure can see it.
    Aborted {
        code: String,
        error: String,
    },
    Complete {
        total_tokens: Option<i32>,
    },
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "snake_case")]
enum NotificationData {
    Log {
        message: String,
    },
    Progress {
        progress: f64,
        total: Option<f64>,
        message: Option<String>,
    },
}

pub enum RunMode {
    Normal,
    Plan,
}

struct HistoryManager {
    history_file: PathBuf,
    old_history_file: PathBuf,
}

impl HistoryManager {
    fn new() -> Self {
        Self {
            history_file: Paths::state_dir().join("history.txt"),
            old_history_file: Paths::config_dir().join("history.txt"),
        }
    }

    fn load(
        &self,
        editor: &mut rustyline::Editor<BioRouterCompleter, rustyline::history::DefaultHistory>,
    ) {
        if let Some(parent) = self.history_file.parent() {
            if !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    eprintln!("Warning: Failed to create history directory: {}", e);
                }
            }
        }

        let history_files = [&self.history_file, &self.old_history_file];
        if let Some(file) = history_files.iter().find(|f| f.exists()) {
            if let Err(err) = editor.load_history(file) {
                eprintln!("Warning: Failed to load command history: {}", err);
            }
        }
    }

    fn save(
        &self,
        editor: &mut rustyline::Editor<BioRouterCompleter, rustyline::history::DefaultHistory>,
    ) {
        if let Err(err) = editor.save_history(&self.history_file) {
            eprintln!("Warning: Failed to save command history: {}", err);
        } else if self.old_history_file.exists() {
            if let Err(err) = std::fs::remove_file(&self.old_history_file) {
                eprintln!("Warning: Failed to remove old history file: {}", err);
            }
        }
    }
}

pub struct CliSession {
    agent: Agent,
    messages: Conversation,
    session_id: String,
    completion_cache: Arc<std::sync::RwLock<CompletionCache>>,
    debug: bool,
    run_mode: RunMode,
    scheduled_job_id: Option<String>, // ID of the scheduled job that triggered this session
    max_turns: Option<u32>,
    edit_mode: Option<EditMode>,
    retry_config: Option<RetryConfig>,
    output_format: String,
    /// Set when the last turn ended without doing its work (a provider failure, a
    /// tool loop, a worker timeout). Drives the process exit code and the
    /// `status` field in `--output-format json`, both of which used to claim
    /// success no matter what happened.
    last_abort: Option<TurnAbortCode>,
    /// #31: the private per-run session store backing a `--no-session` run.
    /// Held only so the temp directory outlives the session; dropped (and the
    /// store deleted, best-effort) with the session.
    ephemeral_store_dir: Option<tempfile::TempDir>,
    /// #31: the final session row, captured just before the private
    /// `--no-session` store is closed. `headless()`/`interactive()` tear the
    /// store down before `cli.rs` logs session completion, and that logging
    /// queries the (by then closed) store — without this snapshot every
    /// `--no-session` run reported zero tokens and zero messages.
    final_session_snapshot: Option<biorouter::session::Session>,
}

// Cache structure for completion data
struct CompletionCache {
    last_updated: Instant,
}

impl CompletionCache {
    fn new() -> Self {
        Self {
            last_updated: Instant::now(),
        }
    }
}

pub enum PlannerResponseType {
    Plan,
    ClarifyingQuestions,
}

/// Decide if the planner's reponse is a plan or a clarifying question
///
/// This function is called after the planner has generated a response
/// to the user's message. The response is either a plan or a clarifying
/// question.
pub async fn classify_planner_response(
    message_text: String,
    provider: Arc<dyn Provider>,
) -> Result<PlannerResponseType> {
    let prompt = format!("The text below is the output from an AI model which can either provide a plan or list of clarifying questions. Based on the text below, decide if the output is a \"plan\" or \"clarifying questions\".\n---\n{message_text}");

    // Generate the description
    let message = Message::user().with_text(&prompt);
    let (result, _usage) = provider
        .complete(
            "Reply only with the classification label: \"plan\" or \"clarifying questions\"",
            &[message],
            &[],
        )
        .await?;

    let predicted = result.as_concat_text();
    if predicted.to_lowercase().contains("plan") {
        Ok(PlannerResponseType::Plan)
    } else {
        Ok(PlannerResponseType::ClarifyingQuestions)
    }
}

impl CliSession {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        agent: Agent,
        session_id: String,
        debug: bool,
        scheduled_job_id: Option<String>,
        max_turns: Option<u32>,
        edit_mode: Option<EditMode>,
        retry_config: Option<RetryConfig>,
        output_format: String,
    ) -> Self {
        let messages = agent
            .config
            .session_manager
            .get_session(&session_id, true)
            .await
            .map(|session| session.conversation.unwrap_or_default())
            .unwrap();

        CliSession {
            agent,
            messages,
            session_id,
            completion_cache: Arc::new(std::sync::RwLock::new(CompletionCache::new())),
            debug,
            run_mode: RunMode::Normal,
            scheduled_job_id,
            max_turns,
            edit_mode,
            retry_config,
            output_format,
            last_abort: None,
            ephemeral_store_dir: None,
            final_session_snapshot: None,
        }
    }

    /// #31: adopt the private `--no-session` store's temp directory so it
    /// lives exactly as long as this session.
    pub fn hold_ephemeral_store_dir(&mut self, dir: tempfile::TempDir) {
        self.ephemeral_store_dir = Some(dir);
    }

    /// #31: close the private `--no-session` store at the natural end of a
    /// run — SQLite pool first (releasing the WAL/-shm handles), then the
    /// temp directory. Without the pool close, the deletion only works where
    /// unlinking open files is allowed (Unix); on Windows it fails and the
    /// directory leaks. The `Drop` of the held `TempDir` remains the
    /// best-effort fallback for abnormal exits, but it cannot close the pool
    /// first, so it keeps that platform caveat.
    ///
    /// The final session row is snapshotted BEFORE the pool closes: the
    /// caller in `cli.rs` logs completion telemetry (tokens, message count)
    /// *after* this teardown via [`Self::get_session`], which falls back to
    /// the snapshot once the store is gone — every `--no-session` run used
    /// to log zeros here.
    async fn close_ephemeral_store(&mut self) {
        if let Some(dir) = self.ephemeral_store_dir.take() {
            self.final_session_snapshot = self
                .agent
                .config
                .session_manager
                .get_session(&self.session_id, false)
                .await
                .ok();
            self.agent.config.session_manager.close().await;
            // ⚠ Through the shared helper, which RETRIES. Closing the pool is
            // not enough on Windows: sqlx reaches `sqlite3_close` on a
            // per-connection background thread, so the db/-wal/-shm handles
            // outlive the await and a single removal loses to os error 32.
            // A bare `dir.close()` here leaked the store on every Windows run.
            builder::close_ephemeral_store(Some(dir)).await;
        }
    }

    /// The session store this run actually writes (#31): the private per-run
    /// temp store under `--no-session`, otherwise the shared `sessions.db`.
    /// Error hints derive from this, so they never blame the shared store for
    /// a failure inside a private one.
    fn active_session_store(&self) -> ActiveSessionStore {
        match &self.ephemeral_store_dir {
            Some(dir) => {
                ActiveSessionStore::Private(dir.path().join("sessions").join("sessions.db"))
            }
            None => {
                ActiveSessionStore::Shared(Paths::data_dir().join("sessions").join("sessions.db"))
            }
        }
    }

    /// The abort code of the last turn, if it ended without doing its work.
    pub fn last_abort(&self) -> Option<&TurnAbortCode> {
        self.last_abort.as_ref()
    }

    /// The process exit code this session should end with: 0 when every turn
    /// completed, otherwise the code for the abort that ended the last one.
    pub fn exit_code(&self) -> u8 {
        self.last_abort
            .as_ref()
            .map_or(abort_exit::OK, TurnAbortCode::exit_code)
    }

    pub fn session_id(&self) -> &String {
        &self.session_id
    }

    /// Parse a stdio extension command string into an ExtensionConfig
    /// Format: "ENV1=val1 ENV2=val2 command args..."
    pub fn parse_stdio_extension(extension_command: &str) -> Result<ExtensionConfig> {
        let mut parts: Vec<&str> = extension_command.split_whitespace().collect();
        let mut envs = HashMap::new();

        while let Some(part) = parts.first() {
            if !part.contains('=') {
                break;
            }
            let env_part = parts.remove(0);
            let (key, value) = env_part.split_once('=').unwrap();
            envs.insert(key.to_string(), value.to_string());
        }

        if parts.is_empty() {
            return Err(anyhow::anyhow!("No command provided in extension string"));
        }

        let cmd = parts.remove(0).to_string();

        Ok(ExtensionConfig::Stdio {
            name: String::new(),
            cmd,
            args: parts.iter().map(|s| s.to_string()).collect(),
            envs: Envs::new(envs),
            env_keys: Vec::new(),
            description: biorouter::config::DEFAULT_EXTENSION_DESCRIPTION.to_string(),
            timeout: Some(biorouter::config::DEFAULT_EXTENSION_TIMEOUT),
            bundled: None,
            available_tools: Vec::new(),
        })
    }

    pub fn parse_streamable_http_extension(extension_url: &str) -> ExtensionConfig {
        ExtensionConfig::StreamableHttp {
            name: String::new(),
            uri: extension_url.to_string(),
            envs: Envs::new(HashMap::new()),
            env_keys: Vec::new(),
            headers: HashMap::new(),
            description: biorouter::config::DEFAULT_EXTENSION_DESCRIPTION.to_string(),
            timeout: Some(biorouter::config::DEFAULT_EXTENSION_TIMEOUT),
            bundled: None,
            available_tools: Vec::new(),
        }
    }

    /// Parse builtin extension names (comma-separated) into ExtensionConfigs
    pub fn parse_builtin_extensions(builtin_name: &str) -> Vec<ExtensionConfig> {
        builtin_name
            .split(',')
            .map(|name| {
                let extension_name = name.trim();
                if PLATFORM_EXTENSIONS.contains_key(extension_name) {
                    ExtensionConfig::Platform {
                        name: extension_name.to_string(),
                        bundled: None,
                        description: extension_name.to_string(),
                        available_tools: Vec::new(),
                    }
                } else {
                    ExtensionConfig::Builtin {
                        name: extension_name.to_string(),
                        display_name: None,
                        timeout: None,
                        bundled: None,
                        description: extension_name.to_string(),
                        available_tools: Vec::new(),
                    }
                }
            })
            .collect()
    }

    async fn add_and_persist_extensions(&mut self, configs: Vec<ExtensionConfig>) -> Result<()> {
        for config in configs {
            self.agent
                .add_extension(config)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to start extension: {}", e))?;
        }

        self.agent
            .persist_extension_state(&self.session_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to save extension state: {}", e))?;

        self.invalidate_completion_cache().await;

        Ok(())
    }

    pub async fn add_extension(&mut self, extension_command: String) -> Result<()> {
        let config = Self::parse_stdio_extension(&extension_command)?;
        self.add_and_persist_extensions(vec![config]).await
    }

    pub async fn add_streamable_http_extension(&mut self, extension_url: String) -> Result<()> {
        let config = Self::parse_streamable_http_extension(&extension_url);
        self.add_and_persist_extensions(vec![config]).await
    }

    pub async fn add_builtin(&mut self, builtin_name: String) -> Result<()> {
        let configs = Self::parse_builtin_extensions(&builtin_name);
        self.add_and_persist_extensions(configs).await
    }

    /// Process a single message and get the response.
    ///
    /// `interactive` is the REAL interactivity of the surrounding run (#40):
    /// it drives the headless auto-deny/auto-cancel of approval prompts and
    /// elicitations inside `process_agent_response`. It used to be hardcoded
    /// `false` here, so a classic interactive TTY session started WITH an
    /// initial prompt (`interactive(Some(prompt))`) silently auto-denied
    /// every approval in that first turn.
    pub(crate) async fn process_message(
        &mut self,
        message: Message,
        interactive: bool,
        cancel_token: CancellationToken,
    ) -> Result<()> {
        let cancel_token = cancel_token.clone();
        self.push_message(message);
        self.process_agent_response(interactive, cancel_token)
            .await?;
        Ok(())
    }

    /// Start an interactive session, optionally with an initial message
    pub async fn interactive(&mut self, prompt: Option<String>) -> Result<()> {
        // Default to the full-screen TUI on a real terminal; the classic
        // readline REPL remains available via BIOROUTER_CLI_CLASSIC=1 (and is
        // used automatically when stdout is not a TTY).
        let use_classic =
            std::env::var("BIOROUTER_CLI_CLASSIC").is_ok() || !std::io::stdout().is_terminal();
        if !use_classic {
            self.update_completion_cache().await?;
            let result = tui::run(self, prompt).await;
            self.fire_session_end_hooks("exit").await;
            self.close_ephemeral_store().await;
            return result;
        }

        if let Some(prompt) = prompt {
            let msg = Message::user().with_text(&prompt);
            // #40: this is a genuinely interactive session whose FIRST turn
            // merely arrived as an argument — it must keep its ability to
            // answer approval prompts and elicitations at the terminal.
            self.process_message(msg, true, CancellationToken::default())
                .await?;
        }

        self.update_completion_cache().await?;

        let mut editor = self.create_editor()?;
        let history_manager = HistoryManager::new();
        history_manager.load(&mut editor);

        output::display_greeting();
        print_startup_notices().await;
        loop {
            self.display_context_usage().await?;

            let input = input::get_input(&mut editor)?;
            if matches!(input, InputResult::Exit) {
                break;
            }
            self.handle_input(input, &history_manager, &mut editor)
                .await?;
        }

        self.fire_session_end_hooks("exit").await;
        self.close_ephemeral_store().await;

        println!(
            "Closing session. Session ID: {}",
            console::style(&self.session_id).cyan()
        );

        Ok(())
    }

    /// Run SessionEnd hooks before the process exits. Awaited (not
    /// fire-and-forget) so hooks finish before shutdown; failure-open.
    async fn fire_session_end_hooks(&self, reason: &str) {
        if let Ok(session) = self.get_session().await {
            let hooks = self.agent.hooks_manager();
            // BR-28: shutdown boundary — join any observe-only hook (Notification,
            // Pre/PostCompact, Subagent*) still running from the last turn, so it
            // completes before the process exits instead of being cut off.
            hooks
                .join_fired(biorouter::hooks::FIRE_JOIN_BUDGET_SHUTDOWN)
                .await;
            let mut payload = biorouter::hooks::HookPayload::new(
                biorouter::hooks::HookEvent::SessionEnd,
                &session.id,
                session.working_dir.to_string_lossy(),
            );
            payload.source = Some(reason.to_string());
            hooks
                .dispatch(
                    biorouter::hooks::HookEvent::SessionEnd,
                    Some(reason),
                    &payload,
                    &session.working_dir,
                )
                .await;
        }
    }

    fn create_editor(
        &self,
    ) -> Result<rustyline::Editor<BioRouterCompleter, rustyline::history::DefaultHistory>> {
        let builder = rustyline::Config::builder().completion_type(rustyline::CompletionType::List);
        let builder = match self.edit_mode {
            Some(mode) => builder.edit_mode(mode),
            None => builder.edit_mode(EditMode::Emacs),
        };
        let config = builder.build();
        let mut editor =
            rustyline::Editor::<BioRouterCompleter, rustyline::history::DefaultHistory>::with_config(
                config,
            )?;
        let completer = BioRouterCompleter::new(self.completion_cache.clone());
        editor.set_helper(Some(completer));
        Ok(editor)
    }

    async fn handle_input(
        &mut self,
        input: InputResult,
        history: &HistoryManager,
        editor: &mut rustyline::Editor<BioRouterCompleter, rustyline::history::DefaultHistory>,
    ) -> Result<()> {
        match input {
            InputResult::Message(content) => {
                self.handle_message_input(&content, history, editor).await?;
            }
            InputResult::Exit => unreachable!("Exit is handled in the main loop"),
            InputResult::AddExtension(cmd) => {
                history.save(editor);
                match self.add_extension(cmd.clone()).await {
                    Ok(_) => output::render_extension_success(&cmd),
                    Err(e) => output::render_extension_error(&cmd, &e.to_string()),
                }
            }
            InputResult::AddBuiltin(names) => {
                history.save(editor);
                match self.add_builtin(names.clone()).await {
                    Ok(_) => output::render_builtin_success(&names),
                    Err(e) => output::render_builtin_error(&names, &e.to_string()),
                }
            }
            InputResult::ToggleTheme => {
                history.save(editor);
                self.handle_toggle_theme();
            }
            InputResult::ToggleFullToolOutput => {
                history.save(editor);
                self.handle_toggle_full_tool_output();
            }
            InputResult::SelectTheme(theme_name) => {
                history.save(editor);
                self.handle_select_theme(&theme_name);
            }
            InputResult::Retry => {}
            InputResult::BioRouterMode(mode) => {
                history.save(editor);
                self.handle_biorouter_mode(&mode)?;
            }
            InputResult::Plan(options) => {
                self.handle_plan_mode(options).await?;
            }
            InputResult::EndPlan => {
                self.run_mode = RunMode::Normal;
                output::render_exit_plan_mode();
            }
            InputResult::Clear => {
                history.save(editor);
                self.handle_clear().await?;
            }
            InputResult::Workflow(filepath_opt) => {
                history.save(editor);
                self.handle_workflow(filepath_opt).await;
            }
            InputResult::Compact => {
                history.save(editor);
                self.handle_compact().await?;
            }
            InputResult::Diverge(name) => {
                history.save(editor);
                self.handle_diverge(name).await?;
            }
            InputResult::Rename(name) => {
                history.save(editor);
                self.handle_rename(name).await?;
            }
        }
        Ok(())
    }

    /// Rename the current session from the classic REPL (`/rename <name>`).
    async fn handle_rename(&self, name: String) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            output::render_error("A chat name cannot be empty.");
            return Ok(());
        }
        match self
            .agent
            .config
            .session_manager
            .update(&self.session_id)
            .user_provided_name(name)
            .apply()
            .await
        {
            Ok(()) => output::render_text(
                &format!("Renamed this chat to \"{name}\""),
                Some(output::ACCENT),
                false,
            ),
            Err(e) => output::render_error(&format!("Couldn't rename chat: {e}")),
        }
        Ok(())
    }

    /// Branch the current conversation into a brand-new session (full history
    /// preserved, original untouched) and open it in a fresh Biorouter desktop
    /// window via the `biorouter://diverge` deeplink. Used by the classic CLI.
    async fn handle_diverge(&self, name: Option<String>) -> Result<()> {
        let outcome = self.diverge_and_open(|url| open::that(url), name).await?;
        match outcome.open_error {
            None => output::render_diverge_success(&outcome.new_session_id),
            Some(err) => {
                output::render_diverge_open_failed(&outcome.new_session_id, &outcome.url, &err)
            }
        }
        Ok(())
    }

    /// Core of `/diverge`, shared by the classic CLI and the TUI. Copies the
    /// current session (full history, original untouched), builds the desktop
    /// deeplink, and hands it to `opener`. The opener is injected so this can be
    /// unit-tested without actually launching the GUI. A failure to open the
    /// window is *not* an error — the branch is still persisted — so it is
    /// reported via `DivergeOutcome::open_error` for the caller to surface.
    pub(crate) async fn diverge_and_open<F>(
        &self,
        opener: F,
        name: Option<String>,
    ) -> Result<DivergeOutcome>
    where
        F: FnOnce(&str) -> std::io::Result<()>,
    {
        let manager = &self.agent.config.session_manager;

        // diverge_session branches the conversation with a placeholder-aware,
        // sibling-numbered name (e.g. "Foo (branch 2)") and records lineage.
        // A caller-provided `name` overrides that default. anchor=None → the
        // branch ends at the most recent complete answer.
        let new_session = manager
            .diverge_session(&self.session_id, name, None)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to branch chat: {}", e))?;

        let url = build_diverge_deeplink(&new_session.id, &new_session.working_dir);
        let open_error = opener(&url).err().map(|e| e.to_string());

        Ok(DivergeOutcome {
            new_session_id: new_session.id,
            url,
            open_error,
        })
    }

    async fn handle_message_input(
        &mut self,
        content: &str,
        history: &HistoryManager,
        editor: &mut rustyline::Editor<BioRouterCompleter, rustyline::history::DefaultHistory>,
    ) -> Result<()> {
        match self.run_mode {
            RunMode::Normal => {
                history.save(editor);
                self.push_message(Message::user().with_text(content));

                if let Err(e) = crate::project_tracker::update_project_tracker(
                    Some(content),
                    Some(&self.session_id),
                ) {
                    eprintln!(
                        "Warning: Failed to update project tracker with instruction: {}",
                        e
                    );
                }

                let _provider = self.agent.provider().await?;

                output::show_thinking();
                let start_time = Instant::now();
                self.process_agent_response(true, CancellationToken::default())
                    .await?;
                output::hide_thinking();

                let elapsed = start_time.elapsed();
                let elapsed_str = format_elapsed_time(elapsed);
                println!(
                    "\n{}",
                    console::style(format!("Elapsed time: {}", elapsed_str)).dim()
                );
            }
            RunMode::Plan => self.plan(content).await?,
        }
        Ok(())
    }

    /// Run one plan-mode turn: the whole message list plus `content`, handed to
    /// the planner provider.
    ///
    /// Issue #56 Gate H. This exists so that the *two* entry points into plan
    /// mode — this `RunMode::Plan` arm and `/plan <text>` — share ONE
    /// `get_reasoner` call. They were byte-identical five-line blocks, which
    /// meant the barrier had two call sites and a test could only ever cover
    /// one of them; the other could be changed to pass `Public` and stay green.
    async fn plan(&mut self, content: &str) -> Result<()> {
        let mut plan_messages = self.messages.clone();
        plan_messages.push(Message::user().with_text(content));
        let reasoner = get_reasoner(self.session_classification().await).await?;
        self.plan_with_reasoner_model(plan_messages, reasoner).await
    }

    fn handle_toggle_theme(&self) {
        let current = output::get_theme();
        let new_theme = match current {
            output::Theme::Ansi => {
                println!("Switching to Light theme");
                output::Theme::Light
            }
            output::Theme::Light => {
                println!("Switching to Dark theme");
                output::Theme::Dark
            }
            output::Theme::Dark => {
                println!("Switching to Ansi theme");
                output::Theme::Ansi
            }
        };
        output::set_theme(new_theme);
    }

    fn handle_select_theme(&self, theme_name: &str) {
        let new_theme = match theme_name {
            "light" => {
                println!("Switching to Light theme");
                output::Theme::Light
            }
            "dark" => {
                println!("Switching to Dark theme");
                output::Theme::Dark
            }
            "ansi" => {
                println!("Switching to Ansi theme");
                output::Theme::Ansi
            }
            _ => output::Theme::Dark,
        };
        output::set_theme(new_theme);
    }

    fn handle_toggle_full_tool_output(&self) {
        let enabled = output::toggle_full_tool_output();
        if enabled {
            println!(
                "{}",
                console::style(
                    "✓ Full tool output enabled - tool parameters will no longer be truncated"
                )
                .green()
            );
        } else {
            println!(
                "{}",
                console::style(
                    "✓ Full tool output disabled - tool parameters will be truncated to fit terminal width"
                )
                .dim()
            );
        }
    }

    fn handle_biorouter_mode(&self, mode: &str) -> Result<()> {
        let config = Config::global();
        let mode = match BioRouterMode::from_str(&mode.to_lowercase()) {
            Ok(mode) => mode,
            Err(_) => {
                output::render_error(&format!(
                    "Invalid mode '{}'. Mode must be one of: auto, approve, chat, smart_approve",
                    mode
                ));
                return Ok(());
            }
        };
        config.set_biorouter_mode(mode)?;
        output::biorouter_mode_message(&format!("Biorouter mode set to '{:?}'", mode));
        Ok(())
    }

    async fn handle_plan_mode(&mut self, options: input::PlanCommandOptions) -> Result<()> {
        self.run_mode = RunMode::Plan;
        output::render_enter_plan_mode();

        if options.message_text.is_empty() {
            return Ok(());
        }

        self.plan(&options.message_text).await
    }

    /// Issue #56 Gate H. This chat's stored classification, read fresh from the
    /// row rather than cached: plan mode is reached from the REPL between turns,
    /// so anything sampled earlier could be arbitrarily stale.
    ///
    /// **Fails closed.** A row that cannot be read is not a licence to ship the
    /// transcript elsewhere; the cost of the wrong answer here is that plan mode
    /// asks for a private planner, and the cost of the other wrong answer is the
    /// whole conversation.
    async fn session_classification(&self) -> biorouter::privacy::SessionClassification {
        match self
            .agent
            .config
            .session_manager
            .get_session(&self.session_id, false)
            .await
        {
            Ok(session) => session.privacy_tier,
            Err(e) => {
                tracing::warn!(
                    "could not read this session's privacy tier ({e}); treating it as private"
                );
                biorouter::privacy::SessionClassification::Private
            }
        }
    }

    /// Clear the conversation everywhere: the persisted SQLite conversation,
    /// the DB token counts, and the in-memory `messages`. Shared by the classic
    /// `/clear` and the TUI so neither desyncs the persisted session.
    pub(crate) async fn clear_conversation(&mut self) -> Result<()> {
        self.agent
            .config
            .session_manager
            .replace_conversation(&self.session_id, &Conversation::default())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to clear session: {}", e))?;

        self.agent
            .config
            .session_manager
            .update(&self.session_id)
            .total_tokens(Some(0))
            .input_tokens(Some(0))
            .output_tokens(Some(0))
            .apply()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to reset token counts: {}", e))?;

        self.messages.clear();
        tracing::info!("Chat context cleared by user.");
        Ok(())
    }

    async fn handle_clear(&mut self) -> Result<()> {
        if let Err(e) = self.clear_conversation().await {
            output::render_error(&e.to_string());
            return Ok(());
        }
        output::render_message(
            &Message::assistant().with_text("Chat context cleared.\n"),
            self.debug,
        );
        Ok(())
    }

    /// `/workflow` — capture this chat as a reusable workflow.
    ///
    /// Goes through the same two steps the desktop's "create workflow from this
    /// chat" does, and that is the point: generate, then enrich from the live
    /// session. This used to generate and save straight away, so the identical
    /// conversation produced a workflow with no `extensions:`, no
    /// `knowledge_bases:` and no `author:` here, and a fully populated one in
    /// the app.
    async fn handle_workflow(&mut self, filepath_opt: Option<String>) {
        println!("{}", console::style("Generating Workflow").green());

        output::show_thinking();
        let workflow = self.agent.create_workflow(self.messages.clone()).await;
        output::hide_thinking();

        let mut workflow = match workflow {
            Ok(workflow) => workflow,
            Err(e) => {
                println!(
                    "{}: {:?}",
                    console::style("Failed to generate workflow").red(),
                    e
                );
                return;
            }
        };

        match knowledge_service_for_enrichment() {
            Ok(knowledge) => {
                match biorouter::workflow::service::session_enrichment(
                    &self.agent,
                    &knowledge,
                    &self.session_id,
                    None,
                )
                .await
                {
                    Ok(enrichment) => biorouter::workflow::service::apply_session_enrichment(
                        &mut workflow,
                        enrichment,
                    ),
                    Err(e) => println!(
                        "{}",
                        console::style(format!(
                            "Could not capture this session's setup into the workflow: {e}"
                        ))
                        .yellow()
                    ),
                }
            }
            Err(e) => println!(
                "{}",
                console::style(format!(
                    "Could not read knowledge bases for the workflow: {e}"
                ))
                .yellow()
            ),
        }

        match self.save_workflow(&workflow, filepath_opt.as_deref()) {
            Ok(path) => println!(
                "{}",
                console::style(format!("Saved workflow to {}", path.display())).green()
            ),
            // ⚠ Print the CHAIN and then the draft. `service::save` wraps the
            // validation failure in "refusing to save a workflow that does not
            // validate", and `anyhow`'s `Display` prints only that outermost
            // context — so the user was told the workflow was invalid and never
            // which field, while the only copy of a document that cost a whole
            // model round-trip went out of scope on the next line. Two losses
            // from one arm: the reason, and the work.
            Err(e) => {
                println!(
                    "{}",
                    console::style(format!("Could not save the workflow: {e}")).red()
                );
                for cause in e.chain().skip(1) {
                    println!("{}", console::style(format!("  caused by: {cause}")).red());
                }
                match workflow.to_yaml() {
                    Ok(yaml) => println!(
                        "{}\n{yaml}",
                        console::style(
                            "Nothing was written. Here is the generated workflow so it is not \
                             lost — fix the problem above and save it yourself:"
                        )
                        .yellow()
                    ),
                    Err(render) => println!(
                        "{}",
                        console::style(format!(
                            "Nothing was written, and the draft could not be rendered \
                             either: {render}"
                        ))
                        .red()
                    ),
                }
            }
        }
    }

    async fn handle_compact(&mut self) -> Result<()> {
        let prompt =
            "Are you sure you want to compact this chat? This will condense the message history.";
        let should_summarize = match cliclack::confirm(prompt).initial_value(true).interact() {
            Ok(choice) => choice,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::Interrupted {
                    false
                } else {
                    return Err(e.into());
                }
            }
        };

        if should_summarize {
            self.push_message(Message::user().with_text(COMPACT_TRIGGERS[0]));
            output::show_thinking();
            self.process_agent_response(true, CancellationToken::default())
                .await?;
            output::hide_thinking();
        } else {
            println!("{}", console::style("Compaction cancelled.").yellow());
        }
        Ok(())
    }

    async fn plan_with_reasoner_model(
        &mut self,
        plan_messages: Conversation,
        reasoner: Arc<dyn Provider>,
    ) -> Result<(), anyhow::Error> {
        let plan_prompt = self.agent.get_plan_prompt().await?;
        output::show_thinking();
        let (plan_response, _usage) = reasoner
            .complete(&plan_prompt, plan_messages.messages(), &[])
            .await?;
        output::render_message(&plan_response, self.debug);
        output::hide_thinking();
        let planner_response_type =
            classify_planner_response(plan_response.as_concat_text(), self.agent.provider().await?)
                .await?;

        match planner_response_type {
            PlannerResponseType::Plan => {
                println!();
                let should_act = match cliclack::confirm(
                    "Do you want to clear message history & act on this plan?",
                )
                .initial_value(true)
                .interact()
                {
                    Ok(choice) => choice,
                    Err(e) => {
                        if e.kind() == std::io::ErrorKind::Interrupted {
                            false // If interrupted, set should_act to false
                        } else {
                            return Err(e.into());
                        }
                    }
                };
                if should_act {
                    output::render_act_on_plan();
                    self.run_mode = RunMode::Normal;
                    // set biorouter mode: auto if that isn't already the case
                    let config = Config::global();
                    let curr_biorouter_mode =
                        config.get_biorouter_mode().unwrap_or(BioRouterMode::Auto);
                    if curr_biorouter_mode != BioRouterMode::Auto {
                        config.set_biorouter_mode(BioRouterMode::Auto).unwrap();
                    }

                    // clear the messages before acting on the plan
                    self.messages.clear();
                    // add the plan response as a user message
                    let plan_message = Message::user().with_text(plan_response.as_concat_text());
                    self.push_message(plan_message);
                    // act on the plan
                    output::show_thinking();
                    self.process_agent_response(true, CancellationToken::default())
                        .await?;
                    output::hide_thinking();

                    // Reset run & biorouter mode
                    if curr_biorouter_mode != BioRouterMode::Auto {
                        config.set_biorouter_mode(curr_biorouter_mode)?;
                    }
                } else {
                    // add the plan response (assistant message) & carry the conversation forward
                    // in the next round, the user might wanna slightly modify the plan
                    self.push_message(plan_response);
                }
            }
            PlannerResponseType::ClarifyingQuestions => {
                // add the plan response (assistant message) & carry the conversation forward
                // in the next round, the user will answer the clarifying questions
                self.push_message(plan_response);
            }
        }

        Ok(())
    }

    /// Process a single message and exit.
    ///
    /// A turn that aborted (provider failure, tool loop, worker timeout) returns
    /// `Err(TurnFailed)` — it is **not** a success. `main` downcasts that to pick
    /// the process exit code, and `log_session_completion` stops recording the
    /// run as successful. Before this, a 403'd run exited 0.
    pub async fn headless(&mut self, prompt: String) -> Result<()> {
        let message = Message::user().with_text(&prompt);
        // #40: a headless run has nobody at the terminal — approvals
        // auto-deny and elicitations auto-cancel.
        let result = self
            .process_message(message, false, CancellationToken::default())
            .await;
        self.fire_session_end_hooks("prompt_input_exit").await;
        self.close_ephemeral_store().await;
        result?;
        if let Some(code) = self.last_abort.clone() {
            let message = format!("the turn did not complete: {}", code.wire_code());
            return Err(TurnFailed::new(code, message).into());
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn process_agent_response(
        &mut self,
        interactive: bool,
        cancel_token: CancellationToken,
    ) -> Result<()> {
        self.last_abort = None;
        let is_json_mode = self.output_format == "json";
        let is_stream_json_mode = self.output_format == "stream-json";

        let session_config = SessionConfig {
            id: self.session_id.clone(),
            schedule_id: self.scheduled_job_id.clone(),
            max_turns: self.max_turns,
            max_tool_calls: None,
            budget: None,
            retry_config: self.retry_config.clone(),
            reasoning_effort: None,
        };
        let user_message = self
            .messages
            .last()
            .ok_or_else(|| anyhow::anyhow!("No user message"))?;

        let cancel_token_interrupt = cancel_token.clone();
        let handle = tokio::spawn(async move {
            if ctrl_c().await.is_ok() {
                cancel_token_interrupt.cancel();
            }
        });
        let _drop_handle = AbortOnDropHandle::new(handle);

        // #31/#41: an error CONSTRUCTING the reply (e.g. the initial session-
        // store writes inside `Agent::reply`, before any stream exists) must
        // not bypass the structured-output finalizer — propagating it here
        // left json-mode stdout empty and stream-json without a terminating
        // `complete`. Record the abort, surface the error on stderr (plus the
        // stream-json `error`/`aborted` events), and fall through to the one
        // finalizer, exactly like a mid-stream failure.
        let reply_stream = match self
            .agent
            .reply(
                user_message.clone(),
                session_config.clone(),
                Some(cancel_token.clone()),
            )
            .await
        {
            Ok(reply_stream) => reply_stream,
            Err(e) => {
                self.last_abort = Some(record_reply_failure(
                    &e,
                    &self.output_format,
                    &self.active_session_store(),
                ));
                return self.emit_final_output().await;
            }
        };
        // Merge per-token assistant text deltas into whole messages before
        // rendering. `stream-json` consumers want the raw event granularity,
        // so only coalesce for human-facing output.
        let mut stream = if is_stream_json_mode {
            reply_stream
        } else {
            stream_coalesce::coalesce_text_deltas(reply_stream)
        };

        let mut progress_bars = output::McpSpinners::new();
        let cancel_token_clone = cancel_token.clone();

        use futures::StreamExt;
        loop {
            tokio::select! {
                result = stream.next() => {
                    match result {
                        Some(Ok(AgentEvent::Message(message))) => {
                            if let Some((id, security_prompt)) = find_tool_confirmation(&message) {
                                // #40: a confirmation prompt cannot be answered in a
                                // headless/structured/pipe run — cliclack would pollute
                                // stdout and block ~forever (or die as the opaque
                                // 'Error: not connected'). Auto-deny on stderr and hand
                                // the denial to the agent so the TURN CONTINUES and
                                // json-mode stdout stays a valid document.
                                if let Some(permission) = headless_auto_decision(
                                    interactive,
                                    &self.output_format,
                                    std::io::stdin().is_terminal(),
                                ) {
                                    let reason = security_prompt
                                        .as_deref()
                                        .unwrap_or("a tool call requested approval");
                                    eprintln!(
                                        "Tool call requires interactive approval ({}) but this \
                                         run is non-interactive - denied automatically. Do not \
                                         retry the same call; run interactively (`-s`) in a \
                                         terminal with stdin attached to approve it.",
                                        reason.lines().next().unwrap_or(reason).trim()
                                    );
                                    if is_stream_json_mode {
                                        emit_stream_event(&StreamEvent::Notification {
                                            extension_id: "biorouter".to_string(),
                                            data: NotificationData::Log {
                                                message:
                                                    "tool call denied automatically: approval \
                                                     prompt cannot be answered in a \
                                                     non-interactive run"
                                                        .to_string(),
                                            },
                                        });
                                    }
                                    self.agent
                                        .handle_confirmation_for_session(
                                            &self.session_id,
                                            id,
                                            PermissionConfirmation {
                                                principal_type: PrincipalType::Tool,
                                                permission,
                                            },
                                            // Nobody is at the terminal in a
                                            // non-interactive run — that is the
                                            // whole reason this arm exists — so
                                            // it must not claim a person acted.
                                            // It only ever carries a denial in
                                            // any case.
                                            biorouter::pending_user_action::DecisionAuthority::unproven(),
                                        )
                                        .await;
                                    continue;
                                }
                                let permission = prompt_tool_confirmation(&security_prompt)?;

                                if permission == Permission::Cancel {
                                    // #40: prompt-adjacent status stays off a structured stdout
                                    // (the prompt itself already renders on stderr).
                                    if is_json_mode || is_stream_json_mode {
                                        eprintln!("Tool call cancelled. Returning to chat...");
                                    } else {
                                        output::render_text("Tool call cancelled. Returning to chat...", Some(Color::Yellow), true);
                                    }
                                    let mut response_message = Message::user();
                                    response_message.content.push(MessageContent::tool_response(
                                        id,
                                        Err(ErrorData {
                                            code: ErrorCode::INVALID_REQUEST,
                                            message: std::borrow::Cow::from("Tool call cancelled by user"),
                                            data: None,
                                        }),
                                    ));
                                    self.messages.push(response_message);
                                    cancel_token_clone.cancel();
                                    drop(stream);
                                    break;
                                }
                                self.agent
                                    .handle_confirmation_for_session(
                                        &self.session_id,
                                        id,
                                        PermissionConfirmation {
                                            principal_type: PrincipalType::Tool,
                                            permission,
                                        },
                                        // A person answered `prompt_tool_confirmation`
                                        // at a terminal no model authored.
                                        //
                                        // ⚠ That prompt has a defensive non-tty
                                        // arm which can reach here having
                                        // decided `DenyOnce` with nobody
                                        // present. Harmless, because the gate
                                        // fires only on an ALLOW — but it does
                                        // mean this word is not a proof that a
                                        // person spoke, only that a surface a
                                        // model cannot author was asked.
                                        biorouter::pending_user_action::DecisionAuthority::from_local_human_surface(),
                                    )
                                    .await;
                            } else if let Some((elicitation_id, elicitation_message, schema)) = find_elicitation_request(&message) {
                                // #40/#31: an elicitation cannot be answered when nobody
                                // is at the terminal — same predicate as tool
                                // confirmations. Cancel it with a MODEL-VISIBLE result
                                // (the manager unparks the waiting tool call with
                                // ElicitationAction::Cancel) instead of printing prompt
                                // text into a structured stdout and then blocking on
                                // input that never comes.
                                if headless_auto_decision(
                                    interactive,
                                    &self.output_format,
                                    std::io::stdin().is_terminal(),
                                )
                                .is_some()
                                {
                                    eprintln!(
                                        "An extension requested information interactively ({}) \
                                         but this run is non-interactive - cancelled \
                                         automatically. Run interactively (`-s`) in a terminal \
                                         with stdin attached to answer it.",
                                        elicitation_message
                                            .lines()
                                            .next()
                                            .unwrap_or(&elicitation_message)
                                            .trim()
                                    );
                                    if is_stream_json_mode {
                                        emit_stream_event(&StreamEvent::Notification {
                                            extension_id: "biorouter".to_string(),
                                            data: NotificationData::Log {
                                                message:
                                                    "elicitation cancelled automatically: \
                                                     information request cannot be answered in \
                                                     a non-interactive run"
                                                        .to_string(),
                                            },
                                        });
                                    }
                                    if let Err(e) =
                                        biorouter::action_required_manager::ActionRequiredManager::global()
                                            .submit_cancellation(&self.session_id, elicitation_id)
                                            .await
                                    {
                                        eprintln!(
                                            "Failed to cancel the information request: {}",
                                            e
                                        );
                                    }
                                    continue;
                                }
                                output::hide_thinking();
                                let _ = progress_bars.hide();

                                match elicitation::collect_elicitation_input(&elicitation_message, &schema) {
                                    Ok(Some(user_data)) => {
                                        let user_data_value = serde_json::to_value(user_data)
                                            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                                        let response_message = Message::user()
                                            .with_content(MessageContent::action_required_elicitation_response(
                                                elicitation_id,
                                                user_data_value,
                                            ))
                                            .with_visibility(false, true);
                                        self.messages.push(response_message.clone());
                                        // Elicitation responses return an empty stream - the response
                                        // unblocks the waiting tool call via ActionRequiredManager.
                                        //
                                        // #31/#41: a CONSTRUCTION error here (e.g. persisting the
                                        // response inside Agent::reply) must not `?` out past the
                                        // structured-output finalizer — that left json-mode stdout
                                        // empty and stream-json without a terminating `complete`.
                                        // Record the abort and end the turn through the one
                                        // finalizer, exactly like a mid-stream failure.
                                        // `.err()` consumes the returned (empty)
                                        // stream inside this statement, so no
                                        // agent-borrowing temporary outlives it
                                        // and the failure arm may call &mut self.
                                        let elicitation_reply_err = self
                                            .agent
                                            .reply(response_message, session_config.clone(), Some(cancel_token.clone()))
                                            .await
                                            .err();
                                        if let Some(e) = elicitation_reply_err {
                                            self.last_abort = Some(record_reply_failure(
                                                &e,
                                                &self.output_format,
                                                &self.active_session_store(),
                                            ));
                                            cancel_token_clone.cancel();
                                            drop(stream);
                                            if let Err(e) = self.handle_interrupted_messages(false).await {
                                                eprintln!("Error handling interruption: {}", e);
                                            }
                                            break;
                                        }
                                    }
                                    Ok(None) => {
                                        // Prompt-adjacent status stays off a structured
                                        // stdout (#40), like the confirmation Cancel path.
                                        if is_json_mode || is_stream_json_mode {
                                            eprintln!("Information request cancelled.");
                                        } else {
                                            output::render_text("Information request cancelled.", Some(Color::Yellow), true);
                                        }
                                        cancel_token_clone.cancel();
                                        drop(stream);
                                        break;
                                    }
                                    Err(e) => {
                                        output::render_error(&format!("Failed to collect input: {}", e));
                                        cancel_token_clone.cancel();
                                        drop(stream);
                                        break;
                                    }
                                }
                            } else {
                                log_tool_metrics(&message, &self.messages);
                                self.messages.push(message.clone());

                                if interactive { output::hide_thinking() };
                                let _ = progress_bars.hide();

                                if is_stream_json_mode {
                                    emit_stream_event(&StreamEvent::Message { message: message.clone() });
                                } else if !is_json_mode {
                                    output::render_message(&message, self.debug);
                                }

                                // Issue #56 Gate B, at the terminal. The daemon's
                                // refusal is written for the desktop app: it names
                                // "Settings → Models" and "the model chip in the
                                // composer", neither of which exists here. Follow
                                // it with the two commands that do. On stderr, so a
                                // `--output-format json` stdout stays a document.
                                if is_privacy_turn_refusal(&message) {
                                    eprintln!(
                                        "{}",
                                        privacy::repair_block(
                                            &self.session_id,
                                            &privacy::available_private_models().await,
                                        )
                                    );
                                }
                            }
                        }
                        Some(Ok(AgentEvent::McpNotification((extension_id, notification)))) => {
                            handle_mcp_notification(
                                &extension_id,
                                &notification,
                                &mut progress_bars,
                                is_stream_json_mode,
                                interactive,
                                is_json_mode,
                                self.debug,
                            );
                        }
                        Some(Ok(AgentEvent::HistoryReplaced(updated_conversation))) => {
                            self.messages = updated_conversation;
                        }
                        Some(Ok(AgentEvent::ModelChange { model, mode })) => {
                            if is_stream_json_mode {
                                emit_stream_event(&StreamEvent::ModelChange { model: model.clone(), mode: mode.clone() });
                            } else if self.debug {
                                eprintln!("Model changed to {} in {} mode", model, mode);
                            }
                        }
                        // Issue #56 Gate B: what the turn ran on, and how the
                        // chat is classified. Its audience is the desktop
                        // composer, whose model chip is the thing that was saying
                        // something false; a terminal prints its binding at
                        // start-up and `builder.rs` refuses, before creating any
                        // provider, the chats Gate B would have had to repair.
                        // Reported under `--debug` rather than added to
                        // `StreamEvent`, whose shape is a public contract.
                        //
                        // ⚠ The wording no longer claims privacy is the reason.
                        // The frame arrives on every turn now, so "this chat is
                        // private, so …" would be false on most of them.
                        Some(Ok(AgentEvent::PrivacyProviderPinned {
                            provider,
                            model,
                            privacy_tier,
                            ..
                        })) => {
                            if self.debug {
                                eprintln!(
                                    "Turn ran on {provider} / {model} (chat classified {privacy_tier:?})"
                                );
                            }
                        }
                        // BR-52: token accounting is rendered from the session row
                        // in the CLI; the carried snapshot is informational here.
                        Some(Ok(AgentEvent::TokenUsage(_))) => {}
                        // Advisory pending tool-call hint; the CLI renders the
                        // authoritative tool request when it lands.
                        Some(Ok(AgentEvent::ToolCallPending(_))) => {}
                        // #59: the ids the turn's rows were stored under. The CLI
                        // writes to the same session store it renders from and has
                        // no `expectedMessageIds` to satisfy, so this is inert here.
                        Some(Ok(AgentEvent::MessagesPersisted(_))) => {}
                        Some(Ok(AgentEvent::TurnAborted { code, message })) => {
                            // The human-readable Message was already yielded and
                            // rendered. Record the machine-checkable failure so the
                            // process exits nonzero and `--output-format json` stops
                            // claiming "completed".
                            if is_stream_json_mode {
                                emit_stream_event(&StreamEvent::Aborted {
                                    code: code.wire_code().to_string(),
                                    error: message.clone(),
                                });
                            }
                            self.last_abort = Some(code);
                            break;
                        }
                        Some(Err(e)) => {
                            // #31/#41: record the machine-checkable failure so
                            // `--output-format json` reports status "failed"
                            // (this path used to leave last_abort unset and the
                            // document claimed "completed") and the process
                            // exits nonzero via headless()'s TurnFailed check.
                            // A raw stream error must also be visible to
                            // stream-json consumers as an abort, exactly like
                            // the TurnAborted branch above — before this, only
                            // `error` (advisory) then `complete` were emitted,
                            // and a structured consumer could not detect the
                            // failed turn. All of that lives in
                            // record_reply_failure, shared with the
                            // reply-construction failure paths.
                            self.last_abort = Some(record_reply_failure(
                                &e,
                                &self.output_format,
                                &self.active_session_store(),
                            ));
                            cancel_token_clone.cancel();
                            drop(stream);
                            if let Err(e) = self.handle_interrupted_messages(false).await {
                                eprintln!("Error handling interruption: {}", e);
                            } else if !is_stream_json_mode {
                                // render_error writes to stderr, so this prose can
                                // never contaminate a json-mode stdout document.
                                output::render_error(
                                    "The error above was an exception we were not able to handle.\n\
                                    These errors are often related to connection or authentication\n\
                                    We've removed the chat up to the most recent user message\n\
                                    - depending on the error you may be able to continue",
                                );
                            }
                            break;
                        }
                        None => break,
                    }
                }
                _ = cancel_token_clone.cancelled() => {
                    drop(stream);
                    if let Err(e) = self.handle_interrupted_messages(true).await {
                        eprintln!("Error handling interruption: {}", e);
                    }
                    break;
                }
            }
        }

        self.emit_final_output().await
    }

    /// The ONE structured-output finalizer: every exit from
    /// [`Self::process_agent_response`] — a drained stream, an abort, a raw
    /// stream error, or a reply-construction failure that never produced a
    /// stream — must route through here, so `--output-format json` always
    /// prints exactly one document (status derived from [`Self::last_abort`])
    /// and `stream-json` always terminates with `complete` (#31/#41).
    async fn emit_final_output(&self) -> Result<()> {
        let is_json_mode = self.output_format == "json";
        let is_stream_json_mode = self.output_format == "stream-json";
        if is_json_mode {
            println!("{}", self.build_json_document().await?);
        } else if is_stream_json_mode {
            let total_tokens = self
                .agent
                .config
                .session_manager
                .get_session(&self.session_id, false)
                .await
                .ok()
                .and_then(|s| s.total_tokens);
            emit_stream_event(&StreamEvent::Complete { total_tokens });
        } else {
            println!();
        }

        Ok(())
    }

    /// The ONE document `--output-format json` prints on stdout — split out
    /// of [`Self::emit_final_output`] so tests can assert stdout stays a
    /// single valid document after a failure path ran
    /// [`Self::handle_interrupted_messages`] (#31/#41).
    async fn build_json_document(&self) -> Result<String> {
        // A turn that aborted is NOT "completed". This used to be hardcoded,
        // so a harness parsing the JSON was told a 403'd run succeeded.
        let status = if self.last_abort.is_some() {
            "failed"
        } else {
            "completed"
        };
        let error = self.last_abort.as_ref().map(|c| c.wire_code().to_string());
        let metadata = match self
            .agent
            .config
            .session_manager
            .get_session(&self.session_id, false)
            .await
        {
            Ok(session) => JsonMetadata {
                total_tokens: session.total_tokens,
                status: status.to_string(),
                error,
            },
            Err(_) => JsonMetadata {
                total_tokens: None,
                status: status.to_string(),
                error,
            },
        };
        let json_output = JsonOutput {
            messages: self.messages.messages().to_vec(),
            metadata,
        };
        Ok(serde_json::to_string_pretty(&json_output)?)
    }

    /// Where [`Self::handle_interrupted_messages`]'s prose belongs (#40/#31):
    /// human-facing text mode renders it on stdout as usual; the structured
    /// modes keep stdout a machine-parseable surface — `json` must stay ONE
    /// valid document (printed solely by [`Self::emit_final_output`]) and
    /// `stream-json` a sequence of events — so their prose goes to stderr,
    /// like every other prompt-adjacent status in the #40 work. The messages
    /// the handler *pushes* still reach structured consumers through the
    /// final document's `messages`.
    fn interruption_prose_belongs_on_stderr(output_format: &str) -> bool {
        matches!(output_format, "json" | "stream-json")
    }

    /// Emit one line of interruption prose on the surface
    /// [`Self::interruption_prose_belongs_on_stderr`] picks.
    fn render_interruption_notice(&self, prose: &str) {
        if Self::interruption_prose_belongs_on_stderr(&self.output_format) {
            eprintln!("{prose}");
        } else {
            output::render_message(&Message::assistant().with_text(prose), self.debug);
        }
    }

    async fn handle_interrupted_messages(&mut self, interrupt: bool) -> Result<()> {
        // First, get any tool requests from the last message if it exists
        let tool_requests = self
            .messages
            .last()
            .filter(|msg| msg.role == rmcp::model::Role::Assistant)
            .map_or(Vec::new(), |msg| {
                msg.content
                    .iter()
                    .filter_map(|content| {
                        if let MessageContent::ToolRequest(req) = content {
                            Some((req.id.clone(), req.tool_call.clone()))
                        } else {
                            None
                        }
                    })
                    .collect()
            });

        if !tool_requests.is_empty() {
            // Interrupted during a tool request
            // Create tool responses for all interrupted tool requests
            let mut response_message = Message::user();
            let last_tool_name = tool_requests
                .last()
                .and_then(|(_, tool_call)| {
                    tool_call
                        .as_ref()
                        .ok()
                        .map(|tool| tool.name.to_string().clone())
                })
                .unwrap_or_else(|| "tool".to_string());

            let notification = if interrupt {
                "Interrupted by the user to make a correction".to_string()
            } else {
                "An uncaught error happened during tool use".to_string()
            };
            for (req_id, _) in &tool_requests {
                response_message.content.push(MessageContent::tool_response(
                    req_id.clone(),
                    Err(ErrorData {
                        code: ErrorCode::INTERNAL_ERROR,
                        message: std::borrow::Cow::from(notification.clone()),
                        data: None,
                    }),
                ));
            }
            // TODO(Douwe): update also db
            self.push_message(response_message);
            let prompt = format!(
                "The existing call to {} was interrupted. How would you like to proceed?",
                last_tool_name
            );
            self.push_message(Message::assistant().with_text(&prompt));
            self.render_interruption_notice(&prompt);
        } else {
            // An interruption occurred outside of a tool request-response.
            if let Some(last_msg) = self.messages.last() {
                if last_msg.role == rmcp::model::Role::User {
                    match last_msg.content.first() {
                        Some(MessageContent::ToolResponse(_)) => {
                            // Interruption occurred after a tool had completed but not assistant reply
                            let prompt = "The tool calling loop was interrupted. How would you like to proceed?";
                            self.push_message(Message::assistant().with_text(prompt));
                            self.render_interruption_notice(prompt);
                        }
                        Some(_) => {
                            // A real users message
                            self.messages.pop();
                            let prompt = "Interrupted before the model replied and removed the last message.";
                            self.render_interruption_notice(prompt);
                        }
                        None => {
                            tracing::warn!(
                                "Interrupted with an empty last message; nothing to roll back."
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Update the completion cache with fresh data
    /// This should be called before the interactive session starts
    pub async fn update_completion_cache(&mut self) -> Result<()> {
        let mut cache = self.completion_cache.write().unwrap();
        cache.last_updated = Instant::now();
        Ok(())
    }

    /// Invalidate the completion cache
    /// This should be called when extensions are added or removed
    async fn invalidate_completion_cache(&self) {
        let mut cache = self.completion_cache.write().unwrap();
        cache.last_updated = Instant::now();
    }

    pub fn message_history(&self) -> Conversation {
        self.messages.clone()
    }

    /// Render all past messages from the session history
    pub fn render_message_history(&self) {
        if self.messages.is_empty() {
            return;
        }

        // Print session restored message
        println!(
            "\n{} {} messages loaded into context.",
            console::style("Chat restored:").green().bold(),
            console::style(self.messages.len()).green()
        );

        // Render each message
        for message in self.messages.iter() {
            output::render_message(message, self.debug);
        }

        // Add a visual separator after restored messages
        println!(
            "\n{}\n",
            console::style("──────── New Messages ────────").dim()
        );
    }

    pub async fn get_session(&self) -> Result<biorouter::session::Session> {
        match self
            .agent
            .config
            .session_manager
            .get_session(&self.session_id, false)
            .await
        {
            Ok(session) => Ok(session),
            // #31: the private --no-session store closes before cli.rs logs
            // completion telemetry; answer from the snapshot captured at
            // close so a no-session run reports its real counts, not zeros.
            Err(e) => self.final_session_snapshot.clone().ok_or(e),
        }
    }

    // Get the session's total token usage
    pub async fn get_total_token_usage(&self) -> Result<Option<i32>> {
        let metadata = self.get_session().await?;
        Ok(metadata.total_tokens)
    }

    /// Display enhanced context usage with session totals
    pub async fn display_context_usage(&self) -> Result<()> {
        let provider = self.agent.provider().await?;
        let model_config = provider.get_model_config();
        let context_limit = model_config.context_limit();

        let config = Config::global();
        let show_cost = config
            .get_param::<bool>("BIOROUTER_CLI_SHOW_COST")
            .unwrap_or(false);

        let provider_name = config
            .get_biorouter_provider()
            .unwrap_or_else(|_| "unknown".to_string());

        match self.get_session().await {
            Ok(metadata) => {
                let total_tokens = metadata.total_tokens.unwrap_or(0) as usize;

                output::display_context_usage(total_tokens, context_limit);

                if show_cost {
                    let input_tokens = metadata.input_tokens.unwrap_or(0) as usize;
                    let output_tokens = metadata.output_tokens.unwrap_or(0) as usize;
                    output::display_cost_usage(
                        &provider_name,
                        &model_config.model_name,
                        input_tokens,
                        output_tokens,
                    );
                }
            }
            Err(_) => {
                output::display_context_usage(0, context_limit);
            }
        }

        Ok(())
    }

    /// Save a workflow through the ONE writer.
    ///
    /// ⚠ This used to be a hand-rolled `serde_yaml::to_writer` into
    /// `./workflow.yaml`. That bypassed `service::save`, and with it: the user's
    /// workflow library as the default destination, the slug-and-de-dup filename
    /// rule, the block-scalar reformatting that keeps `instructions` readable,
    /// and the validation that stops an unloadable workflow being written and
    /// reported as saved. With no path given the workflow now lands in the
    /// library, where `list` and the interfaces can see it — the old default
    /// dropped it in whatever directory the CLI happened to be started from.
    fn save_workflow(
        &self,
        workflow: &biorouter::workflow::Workflow,
        filepath_str: Option<&str>,
    ) -> anyhow::Result<PathBuf> {
        let target = match filepath_str {
            Some(path) => biorouter::workflow::service::SaveTarget::Path(PathBuf::from(path)),
            None => biorouter::workflow::service::SaveTarget::Library,
        };
        biorouter::workflow::service::save(workflow, target)
    }

    fn push_message(&mut self, message: Message) {
        self.messages.push(message);
    }
}

fn emit_stream_event(event: &StreamEvent) {
    if let Ok(json) = serde_json::to_string(event) {
        println!("{}", json);
    }
}

/// Print missing required prerequisites and an available update at session
/// start, mirroring `biorouter doctor`. Best-effort; the update probe is
/// bounded (2s) so it never stalls startup.
async fn print_startup_notices() {
    use console::style;
    let missing: Vec<_> = biorouter::system::check_all()
        .into_iter()
        .filter(|d| d.required && !d.installed)
        .collect();
    for d in &missing {
        println!(
            "{} {} {}",
            style("⚠").yellow(),
            style(format!("{} not found.", d.display_name)).yellow(),
            style("Run `biorouter doctor` to set up prerequisites").dim()
        );
    }
    if let Ok(Some(u)) = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        biorouter::system::check_for_update(),
    )
    .await
    {
        if u.update_available {
            println!(
                "{} {} {}",
                style("↑").color256(137).bold(),
                style(format!("Biorouter {} is available", u.latest)).color256(137),
                style(format!(
                    "(you have {}). Install the latest release to upgrade",
                    u.current
                ))
                .dim()
            );
        }
    }
}

/// #40: the auto-decision for a tool-confirmation prompt that cannot be
/// answered, or `None` when the run is genuinely interactive.
///
/// A cliclack prompt in a headless run blocked for ~30 minutes on a keypress
/// that could never come — or, with no terminal at all, died as the opaque
/// `Error: not connected`. Auto-deny exactly when nobody can answer: the run
/// was started non-interactively (`biorouter run` without `-s`), or stdin is
/// not a TTY (piped/redirected). Deny-once is the only safe default; the
/// denial goes back to the model as a tool error so the turn continues.
///
/// `interactive && stdin_is_tty` is authoritative — a structured stdout
/// (`json` / `stream-json`) does NOT force a denial, because every piece of
/// prompt UI (cliclack renders on stderr, the security text is `eprintln!`ed,
/// cancel notices go to stderr in structured modes) stays off stdout, so the
/// document remains valid while a human answers on the terminal. The
/// `output_format` parameter is retained so the matrix test pins exactly
/// that: format must never flip the decision.
///
/// Pure (no I/O) so the full matrix is unit-testable without a pty.
fn headless_auto_decision(
    interactive: bool,
    _output_format: &str,
    stdin_is_tty: bool,
) -> Option<Permission> {
    if !interactive || !stdin_is_tty {
        Some(Permission::DenyOnce)
    } else {
        None
    }
}

/// Prompt user for tool call confirmation, returns the Permission selected
fn prompt_tool_confirmation(security_prompt: &Option<String>) -> Result<Permission> {
    output::hide_thinking();

    // #40 defensive guard: even if a caller reaches this prompt without a
    // usable stdin (headless_auto_decision should have caught it), deny
    // explicitly instead of letting cliclack fail with the unexplained
    // `Error: not connected` (io::ErrorKind::NotConnected on a non-terminal).
    if !std::io::stdin().is_terminal() {
        eprintln!(
            "Tool call requires interactive approval but stdin is not a terminal - denied \
             automatically."
        );
        return Ok(Permission::DenyOnce);
    }

    let prompt = if let Some(security_message) = security_prompt {
        // stderr, where cliclack renders too: security-prompt text must never
        // reach stdout, which may be a structured document or a pipe (#40).
        eprintln!("\n{}", security_message);
        "Do you allow this tool call?".to_string()
    } else {
        "Biorouter would like to call the above tool, do you allow?".to_string()
    };

    let permission_result = if security_prompt.is_none() {
        cliclack::select(prompt)
            .item(Permission::AllowOnce, "Allow", "Allow the tool call once")
            .item(
                Permission::AlwaysAllow,
                "Always Allow",
                "Always allow the tool call",
            )
            .item(Permission::DenyOnce, "Deny", "Deny the tool call")
            .item(
                Permission::Cancel,
                "Cancel",
                "Cancel the AI response and tool call",
            )
            .interact()
    } else {
        cliclack::select(prompt)
            .item(Permission::AllowOnce, "Allow", "Allow the tool call once")
            .item(Permission::DenyOnce, "Deny", "Deny the tool call")
            .item(
                Permission::Cancel,
                "Cancel",
                "Cancel the AI response and tool call",
            )
            .interact()
    };

    match permission_result {
        Ok(p) => Ok(p),
        Err(e) => {
            if e.kind() == std::io::ErrorKind::Interrupted {
                Ok(Permission::Cancel)
            } else {
                Err(e.into())
            }
        }
    }
}

/// Extract tool confirmation request from a message
fn find_tool_confirmation(message: &Message) -> Option<(String, Option<String>)> {
    message.content.iter().find_map(|content| {
        if let MessageContent::ActionRequired(action) = content {
            if let ActionRequiredData::ToolConfirmation { id, prompt, .. } = &action.data {
                return Some((id.clone(), prompt.clone()));
            }
        }
        None
    })
}

/// Extract elicitation request from a message
fn find_elicitation_request(message: &Message) -> Option<(String, String, Value)> {
    message.content.iter().find_map(|content| {
        if let MessageContent::ActionRequired(action) = content {
            if let ActionRequiredData::Elicitation {
                id,
                message,
                requested_schema,
            } = &action.data
            {
                return Some((id.clone(), message.clone(), requested_schema.clone()));
            }
        }
        None
    })
}

/// Handle MCP notification event (logging or progress)
fn handle_mcp_notification(
    extension_id: &str,
    notification: &ServerNotification,
    progress_bars: &mut output::McpSpinners,
    is_stream_json_mode: bool,
    interactive: bool,
    is_json_mode: bool,
    debug: bool,
) {
    match notification {
        ServerNotification::LoggingMessageNotification(log_notif) => {
            let (formatted, subagent_id, notif_type) =
                format_logging_notification(&log_notif.params.data, debug);

            if is_stream_json_mode {
                emit_stream_event(&StreamEvent::Notification {
                    extension_id: extension_id.to_string(),
                    data: NotificationData::Log {
                        message: formatted.clone(),
                    },
                });
            } else {
                display_log_notification(
                    &formatted,
                    subagent_id.as_deref(),
                    notif_type.as_deref(),
                    progress_bars,
                    interactive,
                    is_json_mode,
                );
            }
        }
        ServerNotification::ProgressNotification(prog_notif) => {
            if is_stream_json_mode {
                emit_stream_event(&StreamEvent::Notification {
                    extension_id: extension_id.to_string(),
                    data: NotificationData::Progress {
                        progress: prog_notif.params.progress,
                        total: prog_notif.params.total,
                        message: prog_notif.params.message.clone(),
                    },
                });
            } else {
                progress_bars.update(
                    &prog_notif.params.progress_token.0.to_string(),
                    prog_notif.params.progress,
                    prog_notif.params.total,
                    prog_notif.params.message.as_deref(),
                );
            }
        }
        _ => (),
    }
}

/// Format a logging notification from MCP, returns (formatted_message, subagent_id, notification_type)
fn format_logging_notification(
    data: &Value,
    debug: bool,
) -> (String, Option<String>, Option<String>) {
    match data {
        Value::String(s) => (s.clone(), None, None),
        Value::Object(o) => {
            if let Some(Value::String(msg)) = o.get("message") {
                let subagent_id = o.get("subagent_id").and_then(|v| v.as_str());
                let notification_type = o.get("type").and_then(|v| v.as_str());

                let formatted = match notification_type {
                    Some("subagent_created") | Some("completed") | Some("terminated") => {
                        format!("subagent: {}", msg)
                    }
                    Some("tool_usage") | Some("tool_completed") | Some("tool_error") => {
                        format!("tool: {}", msg)
                    }
                    Some("message_processing") | Some("turn_progress") => {
                        format!("status: {}", msg)
                    }
                    Some("response_generated") => {
                        let config = Config::global();
                        let min_priority = config
                            .get_param::<f32>("BIOROUTER_CLI_MIN_PRIORITY")
                            .ok()
                            .unwrap_or(0.5);

                        if min_priority > 0.1 && !debug {
                            if let Some(response_content) = msg.strip_prefix("Responded: ") {
                                format!("response: {}", safe_truncate(response_content, 100))
                            } else {
                                format!("response: {}", msg)
                            }
                        } else {
                            format!("response: {}", msg)
                        }
                    }
                    _ => msg.to_string(),
                };
                (
                    formatted,
                    subagent_id.map(str::to_string),
                    notification_type.map(str::to_string),
                )
            } else if let Some(Value::String(output)) = o.get("output") {
                let notification_type = o.get("type").and_then(|v| v.as_str()).map(str::to_string);
                (output.to_owned(), None, notification_type)
            } else if let Some(result) = format_task_execution_notification(data) {
                result
            } else {
                (data.to_string(), None, None)
            }
        }
        v => (v.to_string(), None, None),
    }
}

/// Display a logging notification based on its type and context
fn display_log_notification(
    formatted_message: &str,
    subagent_id: Option<&str>,
    notification_type: Option<&str>,
    progress_bars: &mut output::McpSpinners,
    interactive: bool,
    is_json_mode: bool,
) {
    if subagent_id.is_some() {
        if interactive {
            let _ = progress_bars.hide();
            if !is_json_mode {
                println!("{}", console::style(formatted_message).green().dim());
            }
        } else if !is_json_mode {
            progress_bars.log(formatted_message);
        }
    } else if let Some(ntype) = notification_type {
        if ntype == TASK_EXECUTION_NOTIFICATION_TYPE {
            if interactive {
                let _ = progress_bars.hide();
            }
            if !is_json_mode {
                print!("{}", formatted_message);
                std::io::stdout().flush().unwrap();
            }
        } else if ntype == "shell_output" {
            if interactive {
                let _ = progress_bars.hide();
            }
            if !is_json_mode {
                println!("{}", formatted_message);
            }
        }
    } else if output::is_showing_thinking() {
        output::set_thinking_message(&formatted_message.to_string());
    } else {
        progress_bars.log(formatted_message);
    }
}

/// Log tool request/response metrics
fn log_tool_metrics(message: &Message, messages: &Conversation) {
    for content in &message.content {
        if let MessageContent::ToolRequest(tool_request) = content {
            if let Ok(tool_call) = &tool_request.tool_call {
                tracing::info!(
                    counter.biorouter.tool_calls = 1,
                    tool_name = %tool_call.name,
                    "Tool call started"
                );
            }
        }
        if let MessageContent::ToolResponse(tool_response) = content {
            let tool_name = messages
                .iter()
                .rev()
                .find_map(|msg| {
                    msg.content.iter().find_map(|c| {
                        if let MessageContent::ToolRequest(req) = c {
                            if req.id == tool_response.id {
                                req.tool_call.as_ref().ok().map(|tc| tc.name.clone())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or_else(|| "unknown".to_string().into());

            let result_status = if tool_response.tool_result.is_ok() {
                "success"
            } else {
                "error"
            };
            tracing::info!(
                counter.biorouter.tool_completions = 1,
                tool_name = %tool_name,
                result = %result_status,
                "Tool call completed"
            );
        }
    }
}

/// The machine-checkable abort code for a raw agent error — the
/// classification shared by the reply-construction failure paths (primary
/// and elicitation-response replies) and the mid-stream `Err` branch, so all
/// report identically in structured output. Classified by downcast in
/// `biorouter` (#31): a session-store (sqlx) failure gets its own
/// `session_store_failure` wire code and exit code instead of blaming the
/// provider for a local db problem.
fn reply_abort_code(e: &anyhow::Error) -> TurnAbortCode {
    biorouter::agents::turn_abort::classify_agent_error(e)
}

/// Record a failed reply uniformly (#31/#41): stderr + the stream-json
/// `error` event via [`handle_agent_error`], then the machine-checkable
/// `aborted` event. Returns the classified abort code — the caller MUST
/// store it in `last_abort` and fall through to the one finalizer
/// (`emit_final_output`); never bypass that with `?`, which left json-mode
/// stdout empty and stream-json without a terminating `complete`. Shared by
/// the reply-construction failure paths (primary AND elicitation-response
/// replies) and the mid-stream `Err` branch. A free function (not `&mut
/// self`) so it can run while the reply stream still borrows the agent.
#[must_use]
fn record_reply_failure(
    e: &anyhow::Error,
    output_format: &str,
    store: &ActiveSessionStore,
) -> TurnAbortCode {
    let is_stream_json_mode = output_format == "stream-json";
    handle_agent_error(e, is_stream_json_mode, store);
    let code = reply_abort_code(e);
    if is_stream_json_mode {
        emit_stream_event(&StreamEvent::Aborted {
            code: code.wire_code().to_string(),
            error: e.to_string(),
        });
    }
    code
}

/// Handle and display an agent error
fn handle_agent_error(e: &anyhow::Error, is_stream_json_mode: bool, store: &ActiveSessionStore) {
    let error_msg = e.to_string();

    if is_stream_json_mode {
        emit_stream_event(&StreamEvent::Error {
            error: error_msg.clone(),
        });
    }

    if e.downcast_ref::<biorouter::providers::errors::ProviderError>()
        .map(|provider_error| {
            matches!(
                provider_error,
                biorouter::providers::errors::ProviderError::ContextLengthExceeded(_)
            )
        })
        .unwrap_or(false)
    {
        if !is_stream_json_mode {
            output::render_text(
                "Compaction requested. Should have happened in the agent!",
                Some(Color::Yellow),
                true,
            );
        }
        warn!("Compaction requested. Should have happened in the agent!");
    }

    if !is_stream_json_mode {
        eprintln!("Error: {}", error_msg);
    }
    // #31: a raw SQLite dump ("error returned from database…") tells the user
    // nothing actionable. Name the ACTIVE store and its likely causes on
    // stderr.
    if let Some(hint) = session_store_error_hint(&error_msg, store) {
        eprintln!("{}", hint);
    }
}

/// The session store a run actually writes: the shared
/// `<data_dir>/sessions/sessions.db`, or the private per-run temp store a
/// `--no-session` run gets (#31). Carried into error hints so they name the
/// store that actually failed instead of always blaming the shared one.
#[derive(Debug, Clone, PartialEq)]
enum ActiveSessionStore {
    Shared(PathBuf),
    Private(PathBuf),
}

/// A human-readable stderr hint for session-store failures, or `None` when the
/// error is not store-shaped. Pure so the matching is unit-testable. The
/// advice follows the ACTIVE store: a run already on its private
/// `--no-session` store cannot be contending with other biorouter processes,
/// and recommending `--no-session` to it would be nonsense.
fn session_store_error_hint(error_msg: &str, store: &ActiveSessionStore) -> Option<String> {
    let store_shaped = error_msg.contains("error returned from database")
        || error_msg.contains("sessions.db")
        || error_msg.contains("messages.msg_uid");
    if !store_shaped {
        return None;
    }
    Some(match store {
        ActiveSessionStore::Shared(path) => format!(
            "The session store ({}) rejected an operation. If another biorouter \
             process (the desktop app, biorouterd, or a concurrent run) is using \
             the same store, retry once it is idle, or use `--no-session` for a \
             fully isolated run.",
            path.display()
        ),
        ActiveSessionStore::Private(path) => format!(
            "This run's private session store ({}) rejected an operation. The \
             store is exclusive to this `--no-session` run, so another biorouter \
             process is not the cause. Check for a full disk or the OS temp \
             directory being cleaned mid-run, then retry.",
            path.display()
        ),
    })
}

/// The planner provider for plan mode.
///
/// Issue #56 Gate H: `session` is the classification of the chat whose whole
/// message list is about to be handed to this provider. It is a parameter rather
/// than something read here because this function has no session — and it is not
/// optional, so a future caller cannot forget it by omission.
async fn get_reasoner(
    session: biorouter::privacy::SessionClassification,
) -> Result<Arc<dyn Provider>, anyhow::Error> {
    use biorouter::model::ModelConfig;
    use biorouter::providers::create;

    let config = Config::global();

    // Try planner-specific provider first, fallback to default provider
    let provider = if let Ok(provider) = config.get_param::<String>("BIOROUTER_PLANNER_PROVIDER") {
        provider
    } else {
        println!("WARNING: BIOROUTER_PLANNER_PROVIDER not found. Using default provider...");
        config
            .get_biorouter_provider()
            .expect("No provider configured. Run 'biorouter configure' first")
    };

    // Try planner-specific model first, fallback to default model
    let model = if let Ok(model) = config.get_param::<String>("BIOROUTER_PLANNER_MODEL") {
        model
    } else {
        println!("WARNING: BIOROUTER_PLANNER_MODEL not found. Using default model...");
        config
            .get_biorouter_model()
            .expect("No model configured. Run 'biorouter configure' first")
    };

    let model_config =
        ModelConfig::new_with_context_env(model, Some("BIOROUTER_PLANNER_CONTEXT_LIMIT"))?;
    let reasoner = create(&provider, model_config).await?;

    // Issue #56 Gate H. AFTER `create`, because the tier is a property of what
    // this instance actually resolved and not of the name that was asked for —
    // `create` can hand back a composite whose lead is somebody else entirely.
    // Constructing a provider discloses nothing; `plan_with_reasoner_model`,
    // which hands it the whole message list, is what would — and it is
    // downstream of this `?` on the ONE path (`Session::plan`) that both plan
    // entry points now share.
    biorouter::privacy::assert_alt_provider_allowed(
        "plan mode",
        reasoner.as_ref(),
        session,
        "BIOROUTER_PLANNER_PROVIDER",
    )?;

    Ok(reasoner)
}

/// Format elapsed time duration
/// Shows seconds if less than 60, otherwise shows minutes:seconds
fn format_elapsed_time(duration: std::time::Duration) -> String {
    let total_secs = duration.as_secs();
    if total_secs < 60 {
        format!("{:.2}s", duration.as_secs_f64())
    } else {
        let minutes = total_secs / 60;
        let seconds = total_secs % 60;
        format!("{}m {:02}s", minutes, seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A `CliSession` over a private store rooted at `manager`'s directory,
    /// with its session row already created — the shape of a `--no-session`
    /// run, minus extensions and provider.
    async fn test_cli_session(
        manager: std::sync::Arc<biorouter::session::SessionManager>,
        output_format: &str,
    ) -> CliSession {
        use biorouter::agents::AgentConfig;
        use biorouter::config::permission::PermissionManager;
        use biorouter::session::session_manager::SessionType;

        let session = manager
            .create_session(
                std::env::temp_dir(),
                "CLI Test Session".to_string(),
                SessionType::Hidden,
            )
            .await
            .expect("create the test session row");
        let agent = Agent::with_config(AgentConfig::new(
            manager,
            PermissionManager::instance(),
            None,
            BioRouterMode::Auto,
        ));
        CliSession::new(
            agent,
            session.id,
            false,
            None,
            None,
            None,
            None,
            output_format.to_string(),
        )
        .await
    }

    /// #40: the full decision matrix for tool-confirmation prompts.
    /// `interactive && stdin_is_tty` is authoritative: those runs keep the
    /// prompt regardless of output format (all prompt UI renders on stderr,
    /// so a structured stdout stays a valid document). Everything else
    /// auto-denies so a headless run cannot block on a keypress that will
    /// never come (the ~30-minute hang) or die as `Error: not connected`.
    #[test]
    fn headless_auto_decision_covers_the_full_matrix() {
        // Interactive TTY runs keep the prompt — for EVERY output format.
        // Structured stdout must never flip an answerable prompt into a
        // silent denial; the prompt UI lives on stderr.
        for output_format in ["text", "json", "stream-json"] {
            assert_eq!(
                headless_auto_decision(true, output_format, true),
                None,
                "interactive TTY must prompt, not auto-deny (format={output_format})"
            );
        }

        // Non-interactive (headless `biorouter run -t ...`): deny.
        assert_eq!(
            headless_auto_decision(false, "text", true),
            Some(Permission::DenyOnce)
        );
        // No TTY on stdin (piped/redirected/CI): deny, even if "interactive".
        assert_eq!(
            headless_auto_decision(true, "text", false),
            Some(Permission::DenyOnce)
        );
        assert_eq!(
            headless_auto_decision(true, "json", false),
            Some(Permission::DenyOnce)
        );

        // Fully headless combinations: still exactly DenyOnce (never Cancel —
        // the turn must CONTINUE with the denial as a tool error).
        for output_format in ["text", "json", "stream-json"] {
            for stdin_is_tty in [true, false] {
                let decision = headless_auto_decision(false, output_format, stdin_is_tty);
                assert_eq!(
                    decision,
                    Some(Permission::DenyOnce),
                    "non-interactive must always auto-deny (format={output_format}, tty={stdin_is_tty})"
                );
            }
        }
    }

    /// #40: `interactive(Some(prompt))` — a classic interactive TTY session
    /// that merely STARTS with a prompt argument — must run that first turn
    /// with `interactive = true` (which `process_message` now threads through
    /// instead of hardcoding `false`). The matrix rows below pin what each
    /// wiring produces on a real TTY: the true the entry point now passes
    /// keeps the prompt; the false it used to hardcode auto-denied approvals
    /// and auto-cancelled elicitations in that first turn.
    #[test]
    fn initial_prompt_turn_keeps_interactivity() {
        for output_format in ["text", "json", "stream-json"] {
            // interactive(Some(prompt)) => process_message(msg, true, ..):
            assert_eq!(
                headless_auto_decision(true, output_format, true),
                None,
                "an interactive session with an initial prompt must keep its \
                 prompts (format={output_format})"
            );
            // the old hardcoded false on the SAME run:
            assert_eq!(
                headless_auto_decision(false, output_format, true),
                Some(Permission::DenyOnce),
                "hardcoding interactive=false forced auto-denial \
                 (format={output_format})"
            );
        }
    }

    /// Issue #56 Task 31. The CLI has to recognise Gate B's refusal to follow it
    /// with a terminal repair, and it recognises it by the shared marker rather
    /// than by a phrase retyped here.
    #[test]
    fn a_gate_b_refusal_is_recognised_by_its_marker_and_ordinary_replies_are_not() {
        let session = biorouter::session::session_manager::Session {
            provider_name: Some("anthropic".into()),
            privacy_tier: biorouter::privacy::SessionClassification::Private,
            ..Default::default()
        };
        let refusal =
            Message::assistant().with_text(biorouter::privacy::refusal::turn_refusal(&session));
        assert!(is_privacy_turn_refusal(&refusal));

        assert!(!is_privacy_turn_refusal(
            &Message::assistant().with_text("Sure, here is the analysis you asked for.")
        ));
        // Not merely "mentions privacy": an assistant that talks ABOUT the
        // feature must not have a repair block stapled to its answer.
        assert!(!is_privacy_turn_refusal(&Message::assistant().with_text(
            "This chat is private, so only a private model may run in it."
        )));
    }

    /// Issue #56 Gate H. CLI plan mode is a documented first-class feature and
    /// a complete private→public transcript leak: `Session::plan` clones the
    /// WHOLE message list and hands it to a provider built from
    /// `BIOROUTER_PLANNER_PROVIDER` (or, failing that, the global default),
    /// which the session row never records. Neither `Agent::update_provider`
    /// nor `Agent::reply` is on that path, so Gates A–F are all blind to it.
    ///
    /// `handle_plan_mode` is driven here rather than `Session::plan` directly
    /// because it is one of the two *entry points* a user reaches — and since
    /// both of them (`/plan <text>` and a message typed while `RunMode::Plan`
    /// is set) now funnel through the single `Session::plan`, driving either
    /// one covers the barrier for both.
    ///
    /// The planner here is a REAL provider — an Ollama-engine instance pointed
    /// at a host that is not this machine, which is exactly how `tier()` decides
    /// Public — reached through `with_config_overrides` rather than by mutating
    /// the process environment. The host is a `.invalid` name, so if the gate
    /// ever stopped refusing, the completion that followed would fail to
    /// resolve rather than reach anybody.
    #[tokio::test]
    async fn cli_plan_mode_refuses_to_ship_a_private_transcript_elsewhere() {
        use biorouter::config::with_config_overrides;
        use biorouter::privacy::SessionClassification;
        use biorouter::session::SessionManager;
        use std::collections::HashMap;

        fn planner(host: &str) -> HashMap<String, String> {
            HashMap::from([
                (
                    "BIOROUTER_PLANNER_PROVIDER".to_string(),
                    "ollama".to_string(),
                ),
                ("BIOROUTER_PLANNER_MODEL".to_string(), "qwen3".to_string()),
                ("OLLAMA_HOST".to_string(), host.to_string()),
            ])
        }
        const OFF_MACHINE: &str = "https://api.example-saas.invalid";
        const THIS_MACHINE: &str = "http://localhost:11434";

        // The gate discriminates, in both directions, before anything else is
        // asserted — otherwise a `get_reasoner` that refused unconditionally
        // would pass the interesting half below.
        assert_eq!(
            with_config_overrides(
                planner(OFF_MACHINE),
                get_reasoner(SessionClassification::Public)
            )
            .await
            .expect("a public chat may plan on a public model")
            .tier(),
            biorouter::privacy::ProviderTier::Public,
        );
        assert!(
            with_config_overrides(
                planner(THIS_MACHINE),
                get_reasoner(SessionClassification::Private)
            )
            .await
            .is_ok(),
            "a private chat may plan on a private model"
        );

        let tmp = tempfile::tempdir().unwrap();
        let sm = std::sync::Arc::new(SessionManager::new(tmp.path().to_path_buf()));
        let mut session = test_cli_session(std::sync::Arc::clone(&sm), "text").await;
        sm.update(&session.session_id)
            .raise_privacy(SessionClassification::Private, "test:gate-h")
            .apply()
            .await
            .expect("mark the chat private");

        let err = with_config_overrides(
            planner(OFF_MACHINE),
            session.handle_plan_mode(input::PlanCommandOptions {
                message_text: "summarise".to_string(),
            }),
        )
        .await
        .expect_err("plan mode must refuse to ship a private transcript to a public model");
        let err = err.to_string();
        assert!(
            err.contains("BIOROUTER_PLANNER_PROVIDER"),
            "the refusal has to name the knob that fixes it, got: {err}"
        );
        assert!(
            err.to_lowercase().contains("private"),
            "the refusal has to say why, got: {err}"
        );
    }

    /// #31: a real session-store failure (here: the pool is closed, the same
    /// sqlx error shape a broken/locked db produces inside `Agent::reply`)
    /// must classify as `SessionStore` — its own wire code and exit code —
    /// while non-store errors keep the provider classification. This is the
    /// classification `record_reply_failure` stamps into `last_abort`, so
    /// json/stream-json `error` codes and the process exit code stop blaming
    /// the provider for a local db problem.
    #[tokio::test]
    async fn store_failures_get_their_own_abort_code() {
        use biorouter::session::SessionManager;

        let tmp = tempfile::tempdir().unwrap();
        let sm = SessionManager::new(tmp.path().to_path_buf());
        let session = sm
            .create_session(
                tmp.path().to_path_buf(),
                "AbortCode".to_string(),
                biorouter::session::session_manager::SessionType::Hidden,
            )
            .await
            .unwrap();
        sm.close().await;

        let store_error = sm
            .add_message(&session.id, &Message::user().with_text("x"))
            .await
            .expect_err("a closed pool must fail the write");
        let code = reply_abort_code(&store_error);
        assert_eq!(code, TurnAbortCode::SessionStore);
        assert_eq!(code.wire_code(), "session_store_failure");
        assert_eq!(
            code.exit_code(),
            biorouter::agents::turn_abort::exit::SESSION_STORE
        );

        // Anything without a sqlx error in its chain keeps the provider
        // classification.
        assert!(matches!(
            reply_abort_code(&anyhow::anyhow!("403 Forbidden")),
            TurnAbortCode::ProviderFailure { .. }
        ));
    }

    /// #31: a session-store failure must gain an actionable stderr hint that
    /// names the ACTIVE store; unrelated errors must not.
    #[test]
    fn session_store_errors_get_a_named_store_hint() {
        let db_error = "error returned from database: (code: 2067) UNIQUE constraint failed: \
             messages.session_id, messages.msg_uid";

        let shared =
            ActiveSessionStore::Shared(PathBuf::from("/data/biorouter/sessions/sessions.db"));
        let hint =
            session_store_error_hint(db_error, &shared).expect("a database error is store-shaped");
        assert!(
            hint.contains("/data/biorouter/sessions/sessions.db"),
            "hint must name the shared store"
        );
        assert!(
            hint.contains("--no-session"),
            "the shared-store hint must offer the isolated-run escape"
        );

        assert_eq!(
            session_store_error_hint("Request failed: 403 Forbidden", &shared),
            None,
            "provider errors are not store-shaped"
        );
    }

    /// #31: a run already on its private `--no-session` store must get a hint
    /// naming THAT store — not the shared sessions.db it never touched, and
    /// not a recommendation to use the flag it is already using.
    #[test]
    fn private_store_errors_name_the_private_store() {
        let db_error = "error returned from database: (code: 2067) UNIQUE constraint failed: \
             messages.session_id, messages.msg_uid";

        let private = ActiveSessionStore::Private(PathBuf::from(
            "/tmp/biorouter-no-session-abc/sessions/sessions.db",
        ));
        let hint =
            session_store_error_hint(db_error, &private).expect("a database error is store-shaped");
        assert!(
            hint.contains("/tmp/biorouter-no-session-abc/sessions/sessions.db"),
            "hint must name the private per-run store, got: {hint}"
        );
        assert!(
            !hint.contains("use `--no-session`"),
            "the private-store hint must not recommend the flag the run already uses"
        );
        assert!(
            !hint.contains("another biorouter process (") && !hint.contains("desktop app"),
            "the private store is exclusive to this run; contention advice is wrong: {hint}"
        );
    }

    /// #40/#31: interruption prose belongs on stderr exactly in the
    /// structured modes — `json` stdout must stay ONE valid document and
    /// `stream-json` a sequence of events — while text mode keeps rendering
    /// it on stdout for humans.
    #[test]
    fn interruption_prose_routes_by_output_format() {
        assert!(!CliSession::interruption_prose_belongs_on_stderr("text"));
        assert!(CliSession::interruption_prose_belongs_on_stderr("json"));
        assert!(CliSession::interruption_prose_belongs_on_stderr(
            "stream-json"
        ));
    }

    /// #31: the elicitation-response failure path (record the abort, cancel,
    /// `handle_interrupted_messages`, finalizer) must leave a json-mode
    /// stdout holding a single valid document. The prose the handler used to
    /// print to stdout in every mode now goes to stderr in structured modes
    /// (pinned above), and the recovery prompt it pushes reaches structured
    /// consumers through the document's `messages` instead.
    #[tokio::test]
    async fn json_mode_interruption_yields_one_valid_document() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = std::sync::Arc::new(biorouter::session::SessionManager::new(
            tmp.path().to_path_buf(),
        ));
        let mut session = test_cli_session(manager, "json").await;

        // The aftermath of a failed elicitation reply: the abort is recorded
        // and the interruption handler runs over a turn whose last message
        // is a user tool-response (the loop was mid-tool-call).
        session.last_abort = Some(TurnAbortCode::SessionStore);
        let mut tool_response = Message::user();
        tool_response.content.push(MessageContent::tool_response(
            "req-1",
            Err(ErrorData {
                code: ErrorCode::INTERNAL_ERROR,
                message: std::borrow::Cow::from("interrupted"),
                data: None,
            }),
        ));
        session.messages.push(tool_response);
        session.handle_interrupted_messages(false).await.unwrap();

        // The one thing emit_final_output prints on stdout in json mode.
        let document = session.build_json_document().await.unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&document).expect("stdout must be a single valid JSON document");
        assert_eq!(parsed["metadata"]["status"], "failed");
        assert_eq!(parsed["metadata"]["error"], "session_store_failure");
        assert!(
            document.contains("The tool calling loop was interrupted"),
            "the recovery prompt must reach structured consumers via `messages`"
        );
    }

    /// #31: `headless()` closes the private `--no-session` store before
    /// returning, but `cli.rs` logs session completion AFTERWARDS via
    /// `get_session` — which used to query the closed pool and report zero
    /// tokens/messages for every no-session run. The final row is now
    /// snapshotted at close, so completion telemetry sees the real counts.
    #[tokio::test]
    async fn no_session_completion_stats_survive_the_store_close() {
        let tmp = tempfile::tempdir().unwrap();
        let manager = std::sync::Arc::new(biorouter::session::SessionManager::new(
            tmp.path().to_path_buf(),
        ));
        let mut session = test_cli_session(manager.clone(), "text").await;

        // Real activity in the private store: two messages and a token total.
        manager
            .add_message(&session.session_id, &Message::user().with_text("hi"))
            .await
            .unwrap();
        manager
            .add_message(
                &session.session_id,
                &Message::assistant().with_text("hello"),
            )
            .await
            .unwrap();
        manager
            .update(&session.session_id)
            .total_tokens(Some(1234))
            .apply()
            .await
            .unwrap();

        // Tear down exactly as headless() does before cli.rs logs completion.
        session.hold_ephemeral_store_dir(tmp);
        session.close_ephemeral_store().await;

        // The store really is closed and gone...
        assert!(
            manager
                .get_session(&session.session_id, false)
                .await
                .is_err(),
            "the private store must be closed after teardown"
        );
        // ...yet the completion telemetry the caller logs still sees the run.
        let logged = session
            .get_session()
            .await
            .expect("completion logging must still see the run after close");
        assert_eq!(logged.message_count, 2, "real message count, not zero");
        assert_eq!(
            logged.total_tokens,
            Some(1234),
            "real token total, not zero"
        );
    }

    #[test]
    fn test_build_diverge_deeplink_basic() {
        let url = build_diverge_deeplink("20260622_3", std::path::Path::new("/home/u/proj"));
        assert_eq!(
            url,
            "biorouter://diverge?session_id=20260622_3&dir=%2Fhome%2Fu%2Fproj"
        );
        assert!(url.starts_with("biorouter://diverge?"));
    }

    #[test]
    fn test_build_diverge_deeplink_encodes_spaces_and_specials() {
        let url = build_diverge_deeplink(
            "id with space",
            std::path::Path::new("/tmp/My Projects/a&b"),
        );
        assert!(url.contains("session_id=id%20with%20space"));
        assert!(url.contains("dir=%2Ftmp%2FMy%20Projects%2Fa%26b"));
        // The raw ampersand from the path must NOT introduce a 3rd query param.
        assert_eq!(url.matches('&').count(), 1);
    }

    #[test]
    fn test_format_elapsed_time_under_60_seconds() {
        // Test sub-second duration
        let duration = Duration::from_millis(500);
        assert_eq!(format_elapsed_time(duration), "0.50s");

        // Test exactly 1 second
        let duration = Duration::from_secs(1);
        assert_eq!(format_elapsed_time(duration), "1.00s");

        // Test 45.75 seconds
        let duration = Duration::from_millis(45750);
        assert_eq!(format_elapsed_time(duration), "45.75s");

        // Test 59.99 seconds
        let duration = Duration::from_millis(59990);
        assert_eq!(format_elapsed_time(duration), "59.99s");
    }

    #[test]
    fn test_format_elapsed_time_minutes() {
        // Test exactly 60 seconds (1 minute)
        let duration = Duration::from_secs(60);
        assert_eq!(format_elapsed_time(duration), "1m 00s");

        // Test 61 seconds (1 minute 1 second)
        let duration = Duration::from_secs(61);
        assert_eq!(format_elapsed_time(duration), "1m 01s");

        // Test 90 seconds (1 minute 30 seconds)
        let duration = Duration::from_secs(90);
        assert_eq!(format_elapsed_time(duration), "1m 30s");

        // Test 119 seconds (1 minute 59 seconds)
        let duration = Duration::from_secs(119);
        assert_eq!(format_elapsed_time(duration), "1m 59s");

        // Test 120 seconds (2 minutes)
        let duration = Duration::from_secs(120);
        assert_eq!(format_elapsed_time(duration), "2m 00s");

        // Test 605 seconds (10 minutes 5 seconds)
        let duration = Duration::from_secs(605);
        assert_eq!(format_elapsed_time(duration), "10m 05s");

        // Test 3661 seconds (61 minutes 1 second)
        let duration = Duration::from_secs(3661);
        assert_eq!(format_elapsed_time(duration), "61m 01s");
    }

    #[test]
    fn test_format_elapsed_time_edge_cases() {
        // Test zero duration
        let duration = Duration::from_secs(0);
        assert_eq!(format_elapsed_time(duration), "0.00s");

        // Test very small duration (1 millisecond)
        let duration = Duration::from_millis(1);
        assert_eq!(format_elapsed_time(duration), "0.00s");

        // Test fractional seconds are truncated for minute display
        // 60.5 seconds should still show as 1m 00s (not 1m 00.5s)
        let duration = Duration::from_millis(60500);
        assert_eq!(format_elapsed_time(duration), "1m 00s");
    }
}
