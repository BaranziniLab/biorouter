use anyhow::Result;
use biorouter::config::Config;
use biorouter::workflow::Workflow;
use biorouter_mcp::mcp_server_runner::{serve, McpCommand};
use biorouter_mcp::{
    AutoVisualiserRouter, ComputerControllerServer, DeveloperServer, MemoryServer,
};
use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell as ClapShell};

use crate::commands::apps::{handle_apps_list, handle_apps_open, handle_apps_serve};
use crate::commands::bench::agent_generator;
use crate::commands::configure::handle_configure;
use crate::commands::info::handle_info;
use crate::commands::models::{
    handle_models_current, handle_models_list, handle_models_providers, handle_models_set,
};
use crate::commands::project::{handle_project_default, handle_projects_interactive};
use crate::commands::term::{
    handle_term_info, handle_term_init, handle_term_log, handle_term_run, Shell,
};
use crate::commands::workflow::{handle_deeplink, handle_list, handle_open, handle_validate};

use crate::commands::schedule::{
    handle_schedule_add, handle_schedule_cron_help, handle_schedule_list, handle_schedule_remove,
    handle_schedule_run_now, handle_schedule_services_status, handle_schedule_services_stop,
    handle_schedule_sessions,
};
use crate::commands::session::{handle_session_list, handle_session_remove};
use crate::session::{build_session, SessionBuilderConfig};
use crate::workflows::extract_from_cli::extract_workflow_info_from_cli;
use crate::workflows::workflow::{explain_workflow, render_workflow_as_yaml};
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use biorouter_bench::bench_config::BenchRunConfig;
use biorouter_bench::runners::bench_runner::BenchRunner;
use biorouter_bench::runners::eval_runner::EvalRunner;
use biorouter_bench::runners::metric_aggregator::MetricAggregator;
use biorouter_bench::runners::model_runner::ModelRunner;
use std::io::Read;
use std::path::PathBuf;
use tracing::warn;

#[derive(Parser)]
#[command(author, version, display_name = "", about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

/// The real clap command tree, for tests that must assert against the CLI's
/// actual surface rather than a description of it (Task 42b).
///
/// `Cli` stays private; only the built `Command` escapes.
pub fn command_tree() -> clap::Command {
    <Cli as clap::CommandFactory>::command()
}

#[derive(Args, Debug, Clone)]
#[group(required = false, multiple = false)]
pub struct Identifier {
    #[arg(
        short = 'n',
        long,
        value_name = "NAME",
        help = "Name for the chat session (e.g., 'project-x')",
        long_help = "Specify a name for your chat session. When used with --resume, will resume this specific session if it exists."
    )]
    pub name: Option<String>,

    #[arg(
        long = "session-id",
        alias = "id",
        value_name = "SESSION_ID",
        help = "Session ID (e.g., '20250921_143022')",
        long_help = "Specify a session ID directly. When used with --resume, will resume this specific session if it exists."
    )]
    pub session_id: Option<String>,

    #[arg(
        long,
        value_name = "PATH",
        help = "Legacy: Path for the chat session",
        long_help = "Legacy parameter for backward compatibility. Extracts session ID from the file path (e.g., '/path/to/20250325_200615.
jsonl' -> '20250325_200615')."
    )]
    pub path: Option<PathBuf>,
}

/// Session behavior options shared between Session and Run commands
#[derive(Args, Debug, Clone, Default)]
pub struct SessionOptions {
    #[arg(
        long,
        help = "Enable debug output mode with full content and no truncation",
        long_help = "When enabled, shows complete tool responses without truncation and full paths."
    )]
    pub debug: bool,

    #[arg(
        long = "max-tool-repetitions",
        value_name = "NUMBER",
        help = "Maximum number of consecutive identical tool calls allowed",
        long_help = "Set a limit on how many times the same tool can be called consecutively with identical parameters. Helps prevent infinite loops."
    )]
    pub max_tool_repetitions: Option<u32>,

    #[arg(
        long = "max-turns",
        value_name = "NUMBER",
        help = "Maximum number of turns allowed without user input (default: 1000)",
        long_help = "Set a limit on how many turns (iterations) the agent can take without asking for user input to continue."
    )]
    pub max_turns: Option<u32>,
}

/// Extension configuration options shared between Session and Run commands
#[derive(Args, Debug, Clone, Default)]
pub struct ExtensionOptions {
    #[arg(
        long = "with-extension",
        value_name = "COMMAND",
        help = "Add stdio extensions (can be specified multiple times)",
        long_help = "Add stdio extensions from full commands with environment variables. Can be specified multiple times. Format: 'ENV1=val1 ENV2=val2 command args...'",
        action = clap::ArgAction::Append
    )]
    pub extensions: Vec<String>,

    #[arg(
        long = "with-streamable-http-extension",
        value_name = "URL",
        help = "Add streamable HTTP extensions (can be specified multiple times)",
        long_help = "Add streamable HTTP extensions from a URL. Can be specified multiple times. Format: 'url...'",
        action = clap::ArgAction::Append
    )]
    pub streamable_http_extensions: Vec<String>,

    #[arg(
        long = "with-builtin",
        value_name = "NAME",
        help = "Add builtin extensions by name (e.g., 'developer' or multiple: 'developer,github')",
        long_help = "Add one or more builtin extensions that are bundled with Biorouter by specifying their names, comma-separated",
        value_delimiter = ','
    )]
    pub builtins: Vec<String>,
}

/// Input source and workflow options for the run command
#[derive(Args, Debug, Clone, Default)]
pub struct InputOptions {
    /// Path to instruction file containing commands
    #[arg(
        short,
        long,
        value_name = "FILE",
        help = "Path to instruction file containing commands. Use - for stdin.",
        conflicts_with = "input_text",
        conflicts_with = "workflow"
    )]
    pub instructions: Option<String>,

    /// Input text containing commands
    #[arg(
        short = 't',
        long = "text",
        value_name = "TEXT",
        help = "Input text to provide to Biorouter directly",
        long_help = "Input text containing commands for Biorouter. Use this in lieu of the instructions argument.",
        conflicts_with = "instructions",
        conflicts_with = "workflow"
    )]
    pub input_text: Option<String>,

    /// Workflow name or full path to the workflow file
    #[arg(
        short = None,
        long = "workflow",
        value_name = "WORKFLOW_NAME or FULL_PATH_TO_WORKFLOW_FILE",
        help = "Workflow name to get workflow file or the full path of the workflow file (use --explain to see workflow details)",
        long_help = "Workflow name to get workflow file or the full path of the workflow file that defines a custom agent configuration. Use --explain to see the workflow's title, description, and parameters.",
        conflicts_with = "instructions",
        conflicts_with = "input_text"
    )]
    pub workflow: Option<String>,

    /// Additional system prompt to customize agent behavior
    #[arg(
        long = "system",
        value_name = "TEXT",
        help = "Additional system prompt to customize agent behavior",
        long_help = "Provide additional system instructions to customize the agent's behavior",
        conflicts_with = "workflow"
    )]
    pub system: Option<String>,

    #[arg(
        long,
        value_name = "KEY=VALUE",
        help = "Dynamic parameters (e.g., --params username=alice --params channel_name=biorouter-channel)",
        long_help = "Key-value parameters to pass to the workflow file. Can be specified multiple times.",
        action = clap::ArgAction::Append,
        value_parser = parse_key_val,
    )]
    pub params: Vec<(String, String)>,

    /// Additional sub-workflow file paths
    #[arg(
        long = "sub-workflow",
        value_name = "WORKFLOW",
        help = "Sub-workflow name or file path (can be specified multiple times)",
        long_help = "Specify sub-workflows to include alongside the main workflow. Can be:\n  - Workflow names from GitHub (if BIOROUTER_WORKFLOW_GITHUB_REPO is configured)\n  - Local file paths to YAML files\nCan be specified multiple times to include multiple sub-workflows.",
        action = clap::ArgAction::Append
    )]
    pub additional_sub_workflows: Vec<String>,

    /// Show the workflow title, description, and parameters
    #[arg(
        long = "explain",
        help = "Show the workflow title, description, and parameters"
    )]
    pub explain: bool,

    /// Print the rendered workflow instead of running it
    #[arg(
        long = "render-workflow",
        help = "Print the rendered workflow instead of running it."
    )]
    pub render_workflow: bool,
}

/// Output configuration options for the run command
#[derive(Args, Debug, Clone)]
pub struct OutputOptions {
    /// Quiet mode - suppress non-response output
    #[arg(
        short = 'q',
        long = "quiet",
        help = "Quiet mode. Suppress non-response output, printing only the model response to stdout"
    )]
    pub quiet: bool,

    /// Output format (text, json, stream-json)
    #[arg(
        long = "output-format",
        value_name = "FORMAT",
        help = "Output format (text, json, stream-json)",
        default_value = "text",
        value_parser = clap::builder::PossibleValuesParser::new(["text", "json", "stream-json"])
    )]
    pub output_format: String,
}

impl Default for OutputOptions {
    fn default() -> Self {
        Self {
            quiet: false,
            output_format: "text".to_string(),
        }
    }
}

/// Model/provider override options for the run command
#[derive(Args, Debug, Clone, Default)]
pub struct ModelOptions {
    /// Provider to use for this run (overrides environment variable)
    #[arg(
        long = "provider",
        value_name = "PROVIDER",
        help = "Specify the LLM provider to use (e.g., 'openai', 'anthropic')",
        long_help = "Override the BIOROUTER_PROVIDER environment variable for this run. Available providers include openai, anthropic, google, ollama, llamacpp, databricks, and others."
    )]
    pub provider: Option<String>,

    /// Model to use for this run (overrides environment variable)
    #[arg(
        long = "model",
        value_name = "MODEL",
        help = "Specify the model to use (e.g., 'claude-opus-5', 'gpt-5.5')",
        long_help = "Override the BIOROUTER_MODEL environment variable for this run. The model must be supported by the specified provider."
    )]
    pub model: Option<String>,
}

/// Run execution behavior options
#[derive(Args, Debug, Clone, Default)]
pub struct RunBehavior {
    /// Continue in interactive mode after processing input
    #[arg(
        short = 's',
        long = "interactive",
        help = "Continue in interactive mode after processing initial input"
    )]
    pub interactive: bool,

    /// Run without storing a session file
    #[arg(
        long = "no-session",
        help = "Run without storing a session file",
        long_help = "Execute commands without creating or using a session file. Useful for automated runs.",
        conflicts_with_all = ["resume", "name", "path"]
    )]
    pub no_session: bool,

    /// Resume a previous run
    #[arg(
        short,
        long,
        action = clap::ArgAction::SetTrue,
        help = "Resume from a previous run",
        long_help = "Continue from a previous run, maintaining the execution state and context."
    )]
    pub resume: bool,

    /// Scheduled job ID (used internally for scheduled executions)
    #[arg(
        long = "scheduled-job-id",
        value_name = "ID",
        help = "ID of the scheduled job that triggered this execution (internal use)",
        long_help = "Internal parameter used when this run command is executed by a scheduled job. This associates the session with the schedule for tracking purposes.",
        hide = true
    )]
    pub scheduled_job_id: Option<String>,
}

/// The provider and model a NEW session row would run with.
///
/// `build_session` resolves four slots in order — the CLI flag, the provider
/// saved on the session row, a workflow pin, and the global default from
/// `config.yaml` (`session/builder.rs`). Three of them apply here: the row is
/// the one about to be created, so its slot is empty by construction.
///
/// ⚠ The global default is read **here** rather than being passed in, and that
/// placement is the fix rather than an implementation detail. Every call site
/// forgot it — `handle_default_session` and `doctor --fix` passed literal
/// `None`s, `session` passed the flags alone, `run` passed the flags or the
/// workflow pin — so from v1.89.0 to v1.90.2 a fully configured install was
/// told "No provider is configured" by every command that starts a chat.
/// Resolving inside the guard is what makes a future call site unable to
/// reintroduce it.
///
/// `Config::get_param` consults the environment before the file, so
/// `BIOROUTER_PROVIDER=… biorouter` resolves through this same call.
fn resolution_for_a_new_row(
    provider: Option<&str>,
    model: Option<&str>,
) -> (Option<String>, Option<String>) {
    let config = Config::global();
    resolution_with_global_default(
        provider,
        model,
        config.get_biorouter_provider().ok(),
        config.get_biorouter_model().ok(),
    )
}

/// The precedence itself, with the configured default handed in.
///
/// Separated from [`resolution_for_a_new_row`] only so the tests can drive
/// every combination of "typed now / configured / neither" without a config
/// file and without racing the process-global `Config`.
fn resolution_with_global_default(
    provider: Option<&str>,
    model: Option<&str>,
    default_provider: Option<String>,
    default_model: Option<String>,
) -> (Option<String>, Option<String>) {
    (
        provider.map(str::to_string).or(default_provider),
        model.map(str::to_string).or(default_model),
    )
}

/// The refusal text for a run that cannot name a provider or a model, or `None`
/// when the row is safe to create.
///
/// The exit is in the caller so that this half — which is all of the logic —
/// can be unit-tested rather than only observed by killing a process.
fn refusal_for_a_new_row(provider: Option<&str>, model: Option<&str>) -> Option<String> {
    let (provider, model) = resolution_for_a_new_row(provider, model);
    crate::session::unconfigured_precondition(provider.as_deref(), model.as_deref())
}

/// Refuse an unconfigured run BEFORE a session row exists.
///
/// `build_session` is the authority on provider and model resolution, but it
/// runs after this function, and by then the row is already in the store. So a
/// fresh install with no provider left one orphan "CLI Session" behind for
/// every attempt the user made before configuring. Every `create_session` call
/// below is preceded by this check.
///
/// ⚠ The provider saved on the session row is absent from the resolution, and
/// that is not an oversight: the row this is guarding is the one about to be
/// created, so that slot is empty by construction. The paths here that return
/// an EXISTING id do not call this, because such a row can legitimately carry
/// the only provider a resumed chat has.
pub(crate) fn refuse_unconfigured_before_creating_a_row(
    provider: Option<&str>,
    model: Option<&str>,
) -> Result<()> {
    if let Some(text) = refusal_for_a_new_row(provider, model) {
        crate::session::output::render_error(&text);
        std::process::exit(1);
    }
    Ok(())
}

async fn get_or_create_session_id(
    identifier: Option<Identifier>,
    resume: bool,
    no_session: bool,
    provider: Option<&str>,
    model: Option<&str>,
) -> Result<Option<String>> {
    if no_session {
        return Ok(None);
    }

    let session_manager = SessionManager::instance();

    let Some(id) = identifier else {
        return if resume {
            let sessions = session_manager.list_sessions().await?;
            if let Some(latest) = sessions.first() {
                Ok(Some(latest.id.clone()))
            } else {
                eprintln!("No previous chat to resume; starting a new chat.");
                refuse_unconfigured_before_creating_a_row(provider, model)?;
                let session = session_manager
                    .create_session(
                        std::env::current_dir()?,
                        "CLI Session".to_string(),
                        SessionType::User,
                    )
                    .await?;
                Ok(Some(session.id))
            }
        } else {
            refuse_unconfigured_before_creating_a_row(provider, model)?;
            let session = session_manager
                .create_session(
                    std::env::current_dir()?,
                    "CLI Session".to_string(),
                    SessionType::User,
                )
                .await?;
            Ok(Some(session.id))
        };
    };

    if let Some(session_id) = id.session_id {
        Ok(Some(session_id))
    } else if let Some(name) = id.name {
        // Resume by name when possible; if `--resume` was requested but no such
        // session exists, fall back to creating a fresh session with that name
        // (with a warning) instead of erroring out — a missing/typo'd session
        // name or a session originally started with `--no-session` should not be
        // a dead end.
        if resume {
            let sessions = session_manager.list_sessions().await?;
            if let Some(existing) = sessions
                .into_iter()
                .find(|s| s.name == name || s.id == name)
            {
                return Ok(Some(existing.id));
            }
            eprintln!(
                "No existing chat named '{name}' to resume; starting a new chat with that name."
            );
        }

        refuse_unconfigured_before_creating_a_row(provider, model)?;
        let session = session_manager
            .create_session(std::env::current_dir()?, name.clone(), SessionType::User)
            .await?;

        session_manager
            .update(&session.id)
            .user_provided_name(name)
            .apply()
            .await?;

        Ok(Some(session.id))
    } else if let Some(path) = id.path {
        let session_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("Could not extract session ID from path: {:?}", path))?;
        Ok(Some(session_id))
    } else {
        refuse_unconfigured_before_creating_a_row(provider, model)?;
        let session = session_manager
            .create_session(
                std::env::current_dir()?,
                "CLI Session".to_string(),
                SessionType::User,
            )
            .await?;
        Ok(Some(session.id))
    }
}

async fn lookup_session_id(identifier: Identifier) -> Result<String> {
    if let Some(session_id) = identifier.session_id {
        Ok(session_id)
    } else if let Some(name) = identifier.name {
        let session_manager = SessionManager::instance();
        // BR-71: subagent runs are addressable by name too — `list_sessions()`
        // filters them out at the SQL level (User + Scheduled only). The widened
        // lookup lives beside the listing that shows those names, so the two
        // cannot disagree about what is addressable, and so it is testable
        // without this function's `SessionManager::instance()` singleton.
        crate::commands::session::resolve_session_by_name(&session_manager, &name)
            .await?
            .ok_or_else(|| anyhow::anyhow!("No session found with name '{}'", name))
    } else if let Some(path) = identifier.path {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("Could not extract session ID from path: {:?}", path))
    } else {
        Err(anyhow::anyhow!("No identifier provided"))
    }
}

fn parse_key_val(s: &str) -> Result<(String, String), String> {
    match s.split_once('=') {
        Some((key, value)) => Ok((key.to_string(), value.to_string())),
        None => Err(format!("invalid KEY=VALUE: {}", s)),
    }
}

#[derive(Subcommand)]
enum SessionCommand {
    #[command(about = "List all available sessions")]
    List {
        #[arg(
            short,
            long,
            help = "Output format (text, json)",
            default_value = "text"
        )]
        format: String,

        #[arg(
            long = "ascending",
            help = "Sort by date in ascending order (oldest first)",
            long_help = "Sort sessions by date in ascending order (oldest first). Default is descending order (newest first)."
        )]
        ascending: bool,

        #[arg(
            short = 'w',
            short_alias = 'p',
            long = "working_dir",
            help = "Filter sessions by working directory"
        )]
        working_dir: Option<PathBuf>,

        #[arg(short = 'l', long = "limit", help = "Limit the number of results")]
        limit: Option<usize>,

        #[arg(
            long = "subagents",
            help = "Include subagent runs, nested under the session that spawned them. \
                    With this flag --limit counts top-level sessions, not total rows, \
                    and each run is marked live/done (or 'state unknown' when no daemon \
                    can be reached to ask)"
        )]
        subagents: bool,
    },
    #[command(about = "Remove sessions. Runs interactively if no ID, name, or regex is provided.")]
    Remove {
        #[command(flatten)]
        identifier: Option<Identifier>,
        #[arg(
            short = 'r',
            long,
            help = "Regex for removing matched sessions (optional)"
        )]
        regex: Option<String>,
    },
    #[command(about = "Export a session")]
    Export {
        #[command(flatten)]
        identifier: Option<Identifier>,

        #[arg(
            short,
            long,
            help = "Output file path (default: stdout)",
            long_help = "Path to save the exported Markdown. If not provided, output will be sent to stdout"
        )]
        output: Option<PathBuf>,

        #[arg(
            long = "format",
            value_name = "FORMAT",
            help = "Output format (markdown, json, yaml)",
            default_value = "markdown"
        )]
        format: String,
    },
    #[command(about = "Stream a session's live events (requires a running daemon)")]
    Watch {
        /// Session id to observe.
        session_id: String,
        #[arg(
            long,
            help = "Keep watching after the current turn ends (default: exit on Finish/Error)"
        )]
        follow: bool,
    },
    #[command(about = "Send a prompt into a session and stream its turn")]
    Send {
        /// Session id to send to.
        session_id: String,
        /// The prompt text.
        text: String,
        #[arg(
            long,
            help = "Return as soon as the turn starts instead of streaming it"
        )]
        no_wait: bool,
        #[arg(
            long,
            help = "Read the daemon's raw user-action key from the first line of stdin instead of prompting on the controlling terminal"
        )]
        user_action_key_stdin: bool,
    },
    #[command(
        about = "Attach to a running session: render where it is, follow it live, and steer it",
        long_about = "Joins a session that is running RIGHT NOW. Prints the chat \
                      so far, then follows it live; anything you type is delivered to the \
                      running turn (or starts one). Use `session --resume` instead for a \
                      finished transcript. Resuming a live session opens a second agent \
                      on it and the two do not share the daemon's turn lock."
    )]
    Attach {
        /// Session id to attach to.
        session_id: Option<String>,
        #[arg(
            long,
            value_name = "NAME",
            help = "Attach by session name instead of id (refuses if several sessions share it)"
        )]
        name: Option<String>,
        #[arg(
            long = "of",
            value_name = "PARENT_ID",
            help = "Attach to the running subagent of this parent session"
        )]
        of: Option<String>,
        #[arg(long, help = "Observe only; do not read stdin or send anything")]
        read_only: bool,
        #[arg(
            long,
            help = "Read the daemon's raw user-action key from the first line of stdin instead of prompting on the controlling terminal"
        )]
        user_action_key_stdin: bool,
    },
    #[command(about = "Stop the turn a session is running (idempotent)")]
    Cancel {
        /// Session id whose running turn should be stopped.
        session_id: String,
        #[arg(
            long,
            help = "Read the daemon's raw user-action key from the first line of stdin instead of prompting on the controlling terminal"
        )]
        user_action_key_stdin: bool,
    },
    #[command(name = "diagnostics")]
    Diagnostics {
        /// Session identifier for generating diagnostics
        #[command(flatten)]
        identifier: Option<Identifier>,

        /// Output path for the diagnostics zip file (optional, defaults to current directory)
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
    },
    #[command(about = "Rename a saved session")]
    Rename {
        #[command(flatten)]
        identifier: Option<Identifier>,

        /// The new name for the session
        #[arg(
            long = "new-name",
            value_name = "NAME",
            help = "The new name for the session"
        )]
        new_name: String,
    },
    #[command(
        about = "Diverge a saved session into a new one, preserving full history",
        long_about = "Branch a stored chat into a brand-new session. The original is left untouched. Prints the new session id to stdout (resume it with `biorouter session --resume --session-id <ID>`)."
    )]
    Diverge {
        #[command(flatten)]
        identifier: Option<Identifier>,

        /// Optional name for the new branched session
        #[arg(
            long = "branch-name",
            value_name = "NAME",
            help = "Name for the new (branched) session"
        )]
        branch_name: Option<String>,
    },
    /// Issue #56 Task 31 / §12.4. By id, and only by id: `list_sessions`
    /// filters to (`user`, `scheduled`), so a private `Hidden`, `SubAgent` or
    /// `Terminal` chat cannot be picked from any listing the app builds — this
    /// is the only surface that can reach one.
    #[command(
        about = "Declassify a private session so it may run on any model",
        long_about = "Lower a session's privacy classification from private to public, after a \
                      confirmation at the terminal. The change is recorded in the \
                      classification ledger. Works by session id, including for sessions that \
                      no listing shows (subagent runs, --no-session runs, terminal sessions)."
    )]
    Declassify {
        /// Session id to declassify.
        session_id: String,
    },
}

#[derive(Subcommand, Debug)]
enum SchedulerCommand {
    #[command(about = "Add a new scheduled job")]
    Add {
        #[arg(
            long = "schedule-id",
            alias = "id",
            help = "Unique ID for the recurring scheduled job"
        )]
        schedule_id: String,
        #[arg(
            long,
            help = "Cron expression for the schedule",
            long_help = "Cron expression for when to run the job. Examples:\n  '0 * * * *'     - Every hour at minute 0\n  '0 */2 * * *'   - Every 2 hours\n  '@hourly'       - Every hour (shorthand)\n  '0 9 * * *'     - Every day at 9:00 AM\n  '0 9 * * 1'     - Every Monday at 9:00 AM\n  '0 0 1 * *'     - First day of every month at midnight"
        )]
        cron: String,
        #[arg(
            long,
            help = "Workflow source (path to file, or base64 encoded workflow string)"
        )]
        workflow_source: String,
    },
    #[command(about = "List all scheduled jobs")]
    List {},
    #[command(about = "Remove a scheduled job by ID")]
    Remove {
        #[arg(
            long = "schedule-id",
            alias = "id",
            help = "ID of the scheduled job to remove (removes the recurring schedule)"
        )]
        schedule_id: String,
    },
    /// List sessions created by a specific schedule
    #[command(about = "List sessions created by a specific schedule")]
    Sessions {
        /// ID of the schedule
        #[arg(long = "schedule-id", alias = "id", help = "ID of the schedule")]
        schedule_id: String,
        #[arg(short = 'l', long, help = "Maximum number of sessions to return")]
        limit: Option<usize>,
    },
    #[command(about = "Run a scheduled job immediately")]
    RunNow {
        /// ID of the schedule to run
        #[arg(long = "schedule-id", alias = "id", help = "ID of the schedule to run")]
        schedule_id: String,
    },
    /// Check status of scheduler services (deprecated - no external services needed)
    #[command(about = "[Deprecated] Check status of scheduler services")]
    ServicesStatus {},
    /// Stop scheduler services (deprecated - no external services needed)
    #[command(about = "[Deprecated] Stop scheduler services")]
    ServicesStop {},
    /// Show cron expression examples and help
    #[command(about = "Show cron expression examples and help")]
    CronHelp {},
}

#[derive(Subcommand)]
pub enum BenchCommand {
    #[command(name = "init-config", about = "Create a new starter-config")]
    InitConfig {
        #[arg(short, long, help = "filename with extension for generated config")]
        name: String,
    },

    #[command(about = "Run all benchmarks from a config")]
    Run {
        #[arg(
            short,
            long,
            help = "A config file generated by the config-init command"
        )]
        config: PathBuf,
    },

    #[command(about = "List all available selectors")]
    Selectors {
        #[arg(
            short,
            long,
            help = "A config file generated by the config-init command"
        )]
        config: Option<PathBuf>,
    },

    #[command(name = "eval-model", about = "Run an eval of model")]
    EvalModel {
        #[arg(short, long, help = "A serialized config file for the model only.")]
        config: String,
    },

    #[command(name = "exec-eval", about = "run a single eval")]
    ExecEval {
        #[arg(short, long, help = "A serialized config file for the eval only.")]
        config: String,
    },

    #[command(
        name = "generate-leaderboard",
        about = "Generate a leaderboard CSV from benchmark results"
    )]
    GenerateLeaderboard {
        #[arg(
            short,
            long,
            help = "Path to the benchmark directory containing model evaluation results"
        )]
        benchmark_dir: PathBuf,
    },
}

#[derive(Subcommand)]
enum WorkflowCommand {
    /// Install a workflow file (.json/.yaml) into the workflow library
    #[command(about = "Install a workflow into the library")]
    Install {
        #[arg(help = "Path to a workflow .json or .yaml file")]
        path: String,
    },

    /// Validate a workflow file
    #[command(about = "Validate a workflow")]
    Validate {
        /// Workflow name to get workflow file to validate
        #[arg(
            help = "workflow name to get workflow file or full path to the workflow file to validate"
        )]
        workflow_name: String,
    },

    /// Generate a deeplink for a workflow file
    #[command(about = "Generate a deeplink for a workflow")]
    Deeplink {
        /// Workflow name to get workflow file to generate deeplink
        #[arg(
            help = "workflow name to get workflow file or full path to the workflow file to generate deeplink"
        )]
        workflow_name: String,
        /// Workflow parameters in key=value format (can be specified multiple times)
        #[arg(
            short = 'p',
            long = "param",
            value_name = "KEY=VALUE",
            help = "Workflow parameter in key=value format (can be specified multiple times)"
        )]
        params: Vec<String>,
    },

    /// Open a workflow in Biorouter Desktop
    #[command(about = "Open a workflow in Biorouter Desktop")]
    Open {
        /// Workflow name to get workflow file to open
        #[arg(help = "workflow name or full path to the workflow file")]
        workflow_name: String,
        /// Workflow parameters in key=value format (can be specified multiple times)
        #[arg(
            short = 'p',
            long = "param",
            value_name = "KEY=VALUE",
            help = "Workflow parameter in key=value format (can be specified multiple times)"
        )]
        params: Vec<String>,
    },

    /// List available workflows
    #[command(about = "List available workflows")]
    List {
        /// Output format (text, json)
        #[arg(
            long = "format",
            value_name = "FORMAT",
            help = "Output format (text, json)",
            default_value = "text"
        )]
        format: String,

        /// Show verbose information including workflow descriptions
        #[arg(
            short,
            long,
            help = "Show verbose information including workflow descriptions"
        )]
        verbose: bool,
    },
}

#[derive(Subcommand)]
enum ModelsCommand {
    /// Show the configured provider and model
    #[command(about = "Show the configured provider and model")]
    Current {
        #[arg(
            long = "format",
            value_name = "FORMAT",
            default_value = "text",
            value_parser = clap::builder::PossibleValuesParser::new(["text", "json"])
        )]
        format: String,
    },

    /// List available providers
    #[command(about = "List available providers")]
    Providers {
        #[arg(
            long = "format",
            value_name = "FORMAT",
            default_value = "text",
            value_parser = clap::builder::PossibleValuesParser::new(["text", "json"])
        )]
        format: String,
    },

    /// List known models for a provider
    #[command(about = "List known models for a provider")]
    List {
        #[arg(help = "Provider name, for example openai, anthropic, ollama, tetrate, databricks")]
        provider: String,

        #[arg(
            long = "format",
            value_name = "FORMAT",
            default_value = "text",
            value_parser = clap::builder::PossibleValuesParser::new(["text", "json"])
        )]
        format: String,
    },

    /// Set the default provider and model
    #[command(about = "Set the default provider and model")]
    Set {
        #[arg(long = "provider", value_name = "PROVIDER")]
        provider: String,

        #[arg(long = "model", value_name = "MODEL")]
        model: String,
    },

    /// Manage on-disk local models (Llama Server)
    #[command(about = "Inspect and manage downloaded local models (Llama Server)")]
    Local {
        #[command(subcommand)]
        command: LocalModelCommand,
    },
}

#[derive(Subcommand)]
enum LocalModelCommand {
    /// List the local model catalog and what is already downloaded
    #[command(
        about = "List local models and their download state",
        visible_alias = "ls"
    )]
    List {
        #[arg(
            long = "format",
            value_name = "FORMAT",
            default_value = "text",
            value_parser = clap::builder::PossibleValuesParser::new(["text", "json"])
        )]
        format: String,
    },

    /// Download a local model into the shared cache without starting a chat
    #[command(about = "Download (pre-cache) a local model")]
    Pull {
        #[arg(help = "Catalog model name (e.g. qwen3.5-4b) or a raw owner/repo:QUANT HF spec")]
        model: String,
    },

    /// Delete a downloaded local model to reclaim disk space
    #[command(about = "Delete a downloaded local model", visible_alias = "delete")]
    Rm {
        #[arg(help = "Catalog model name (e.g. gemma4) or a raw owner/repo:QUANT HF spec")]
        model: String,
    },
}

#[derive(Subcommand)]
enum ExtensionCommand {
    /// Install an extension from a .brxt bundle (extracts, runs `uv sync`, registers)
    #[command(about = "Install an extension from a .brxt bundle")]
    Install {
        #[arg(help = "Path to a .brxt bundle")]
        path: std::path::PathBuf,
        #[arg(
            long = "env",
            value_name = "KEY=VALUE",
            help = "Set a plain env var (repeatable)"
        )]
        env: Vec<String>,
        /// ⚠ Kept for scripts that already do this; never recommended. The
        /// value is in your shell history and visible to `ps` for the life of
        /// the process. Prefer the interactive prompt (echo off) or
        /// `--secret-stdin`.
        #[arg(
            long = "secret",
            value_name = "KEY=VALUE",
            help = "Set a secret env var (visible to `ps` — prefer the prompt or --secret-stdin)"
        )]
        secret: Vec<String>,
        #[arg(
            long = "secret-stdin",
            help = "Read KEY=VALUE lines from stdin, for unattended runs"
        )]
        secret_stdin: bool,
        #[arg(long = "no-enable", help = "Install without enabling")]
        no_enable: bool,
    },

    /// List configured extensions
    #[command(about = "List configured extensions")]
    List {
        #[arg(
            long = "format",
            value_name = "FORMAT",
            default_value = "text",
            value_parser = clap::builder::PossibleValuesParser::new(["text", "json"])
        )]
        format: String,
    },

    /// Re-enter an installed extension's credentials, with echo off
    #[command(about = "Configure an installed extension's credentials")]
    Configure {
        #[arg(help = "Extension name")]
        name: String,
    },

    /// Remove a configured extension
    #[command(about = "Remove a configured extension")]
    Remove {
        #[arg(help = "Extension name")]
        name: String,
        #[arg(long = "purge", help = "Also delete the installed files on disk")]
        purge: bool,
    },
}

#[derive(Subcommand)]
enum SkillCommand {
    /// Install a skill, or a package of skills, from a .zip or a repository URL
    #[command(about = "Install a skill or skill package from a .zip or a repository URL")]
    Install {
        #[arg(help = "Path to a skill .zip, or a repository URL")]
        source: String,
        #[arg(long = "force", help = "Replace it if already installed")]
        force: bool,
        #[arg(
            long = "as",
            value_name = "bundle|individual",
            help = "How to install a source that could be one package or several separate skills"
        )]
        install_as: Option<String>,
    },

    /// List installed skills with their enabled/disabled state
    #[command(about = "List installed skills and their enabled state")]
    List {},

    /// Re-enable a disabled skill (or bundle) without reinstalling it
    #[command(about = "Enable a skill (remove it from the disabled list)")]
    Enable {
        #[arg(help = "Skill name, bundle name, or slug (as shown by `skill list`)")]
        name: String,
    },

    /// Disable a skill (or bundle) while keeping it installed on disk
    #[command(about = "Disable a skill without removing it")]
    Disable {
        #[arg(help = "Skill name, bundle name, or slug (as shown by `skill list`)")]
        name: String,
    },

    /// Remove an installed skill by slug
    #[command(about = "Remove an installed skill")]
    Remove {
        #[arg(help = "Skill slug (as shown by `skill list`)")]
        slug: String,
    },
}

#[derive(Subcommand)]
enum KnowledgeCommand {
    /// List knowledge bases (hidden ones are dimmed; the primary is marked)
    #[command(about = "List knowledge bases")]
    List {
        #[arg(
            long = "format",
            value_name = "FORMAT",
            default_value = "text",
            value_parser = clap::builder::PossibleValuesParser::new(["text", "json"])
        )]
        format: String,
    },

    /// Show, set, clear, or restore the default primary knowledge base
    #[command(about = "Show or set the primary knowledge base (the --kb-less write target)")]
    Active {
        #[arg(
            long = "set",
            value_name = "ID",
            conflicts_with_all = ["clear", "inherit"],
            help = "Make this base the primary (it must not be hidden)"
        )]
        set: Option<String>,
        #[arg(
            long = "clear",
            conflicts_with = "inherit",
            help = "Clear the primary knowledge base"
        )]
        clear: bool,
        #[arg(
            long = "inherit",
            help = "Drop this scope's preference: a chat follows the machine-wide choice; \
                    machine scope restores Soul"
        )]
        inherit: bool,
        #[arg(
            long = "session",
            value_name = "ID",
            help = "Act on this chat session's primary instead of the machine-wide one"
        )]
        session: Option<String>,
    },

    /// Create a new knowledge base
    #[command(about = "Create a new knowledge base")]
    Create {
        #[arg(help = "Knowledge base id (kebab-case)")]
        id: String,
        #[arg(long = "name", value_name = "NAME", help = "Human-friendly name")]
        name: Option<String>,
        #[arg(
            long = "color",
            value_name = "HEX",
            help = "Accent color, e.g. #cf6d47"
        )]
        color: Option<String>,
    },

    /// Ingest a source (URL, file, or text) into a knowledge base
    #[command(about = "Ingest a source into a knowledge base")]
    Ingest {
        #[arg(
            long = "kb",
            value_name = "ID",
            help = "Target base (defaults to active)"
        )]
        kb: Option<String>,
        #[arg(long = "url", value_name = "URL")]
        url: Option<String>,
        #[arg(long = "file", value_name = "PATH")]
        file: Option<std::path::PathBuf>,
        #[arg(long = "text", value_name = "TEXT")]
        text: Option<String>,
        #[arg(long = "focus", value_name = "HINTS", help = "Optional focus hints")]
        focus: Option<String>,
        #[arg(long = "provider", value_name = "PROVIDER")]
        provider: Option<String>,
        #[arg(long = "model", value_name = "MODEL")]
        model: Option<String>,
    },

    /// Digest one or more chat sessions into a knowledge base
    #[command(
        name = "ingest-conversation",
        about = "Digest chat/session history into a knowledge base"
    )]
    IngestConversation {
        #[arg(
            long = "kb",
            value_name = "ID",
            help = "Target base (defaults to active)"
        )]
        kb: Option<String>,
        #[arg(
            long = "session",
            value_name = "SESSION_ID",
            help = "Session id to ingest (repeatable). Defaults to the most recent session.",
            num_args = 1..,
        )]
        session: Vec<String>,
        #[arg(
            long = "new-kb",
            value_name = "NAME",
            help = "Create a new knowledge base with this display name and ingest into it"
        )]
        new_kb: Option<String>,
        #[arg(long = "focus", value_name = "HINTS", help = "Optional focus hints")]
        focus: Option<String>,
        #[arg(long = "provider", value_name = "PROVIDER")]
        provider: Option<String>,
        #[arg(long = "model", value_name = "MODEL")]
        model: Option<String>,
    },

    /// Lint a knowledge base for orphans, contradictions, and stale sources
    #[command(about = "Lint a knowledge base")]
    Lint {
        #[arg(
            long = "kb",
            value_name = "ID",
            help = "Target base (defaults to active)"
        )]
        kb: Option<String>,
        #[arg(long = "fix", help = "Let the sub-agent repair the findings")]
        fix: bool,
        #[arg(long = "provider", value_name = "PROVIDER")]
        provider: Option<String>,
        #[arg(long = "model", value_name = "MODEL")]
        model: Option<String>,
    },

    /// Hide a knowledge base from the agent (it stays on disk)
    #[command(about = "Hide a knowledge base from the agent")]
    Hide {
        #[arg(help = "Knowledge base id")]
        id: String,
    },

    /// Make a previously hidden knowledge base visible to the agent again
    #[command(about = "Unhide a knowledge base")]
    Unhide {
        #[arg(help = "Knowledge base id")]
        id: String,
    },

    /// Ask a question against a knowledge base
    #[command(about = "Query a knowledge base")]
    Query {
        #[arg(help = "The question to ask")]
        question: String,
        #[arg(
            long = "kb",
            value_name = "ID",
            help = "Target base (defaults to active)"
        )]
        kb: Option<String>,
        #[arg(long = "save", help = "Persist the answer as a knowledge page")]
        save: bool,
        #[arg(long = "provider", value_name = "PROVIDER")]
        provider: Option<String>,
        #[arg(long = "model", value_name = "MODEL")]
        model: Option<String>,
    },
}

#[derive(Subcommand)]
enum AppsCommand {
    /// List installed Biorouter apps
    #[command(about = "List installed Biorouter apps")]
    List {
        /// Emit machine-readable JSON instead of a table
        #[arg(long, help = "Emit machine-readable JSON instead of a table")]
        json: bool,
    },

    /// Open an app in your default browser
    #[command(about = "Open an app in your default browser")]
    Open {
        /// App id (see `biorouter apps list`)
        #[arg(help = "App id (see `biorouter apps list`)")]
        id: String,
    },

    /// Serve an app in the foreground until Ctrl-C
    #[command(about = "Serve an app in the foreground until Ctrl-C")]
    Serve {
        /// App id (see `biorouter apps list`)
        #[arg(help = "App id (see `biorouter apps list`)")]
        id: String,
    },
}

#[derive(Subcommand)]
enum Command {
    /// Configure Biorouter settings
    #[command(about = "Configure Biorouter settings")]
    Configure {},

    /// Display Biorouter configuration information
    #[command(about = "Display Biorouter information")]
    Info {
        /// Show verbose information including current configuration
        #[arg(short, long, help = "Show verbose information including config.yaml")]
        verbose: bool,
    },

    /// Manage system prompts and behaviors
    #[command(about = "Run one of the mcp servers bundled with Biorouter")]
    Mcp {
        #[arg(value_parser = clap::value_parser!(McpCommand))]
        server: McpCommand,
    },

    /// Run Biorouter as an ACP (Agent Client Protocol) agent
    #[command(about = "Run Biorouter as an ACP agent server (stdio by default, or a WebSocket)")]
    Acp {
        /// Add builtin extensions by name
        #[arg(
            long = "with-builtin",
            value_name = "NAME",
            help = "Add builtin extensions by name (e.g., 'developer' or multiple: 'developer,github')",
            long_help = "Add one or more builtin extensions that are bundled with Biorouter by specifying their names, comma-separated",
            value_delimiter = ','
        )]
        builtins: Vec<String>,

        /// Serve over a WebSocket instead of stdio (e.g. for agent-enabled
        /// artifacts). Optional address; defaults to 127.0.0.1:11577.
        #[arg(
            long = "ws",
            value_name = "ADDR",
            num_args = 0..=1,
            default_missing_value = biorouter_acp::server::DEFAULT_WS_ADDR,
            help = "Serve ACP over a WebSocket at ADDR (default 127.0.0.1:11577) instead of stdio"
        )]
        ws: Option<String>,
    },

    /// Start or resume interactive chat sessions
    ///
    /// ⚠ `sessions` is an alias, and it is not cosmetic. `session_watch.rs`'s
    /// own error message told users to run `biorouter sessions watch <id>`, the
    /// BR-71 plan wrote `biorouter sessions …` in roughly forty places, and
    /// `docs/cli/command-reference.md` printed it — while the plural was never a
    /// registered command, so every one of those instructions ended in
    /// `unrecognized subcommand 'sessions'`. Registering it makes the
    /// instructions true rather than making forty documents wrong.
    #[command(
        about = "Start or resume interactive chat sessions",
        visible_aliases = ["s", "sessions"]
    )]
    Session {
        #[command(subcommand)]
        command: Option<SessionCommand>,

        #[command(flatten)]
        identifier: Option<Identifier>,

        /// Resume a previous session
        #[arg(
            short,
            long,
            help = "Resume a previous session (last used or specified by --name/--session-id)",
            long_help = "Continue from a previous session. If --name or --session-id is provided, resumes that specific session. Otherwise resumes the most recently used session."
        )]
        resume: bool,

        /// Show message history when resuming
        #[arg(
            long,
            help = "Show previous messages when resuming a session",
            requires = "resume"
        )]
        history: bool,

        #[command(flatten)]
        session_opts: SessionOptions,

        #[command(flatten)]
        extension_opts: ExtensionOptions,

        // Issue #56 Task 31. `biorouter run` has had these since forever;
        // `biorouter session` did not, so `build_session`'s first precedence
        // slot (`--provider`) was permanently `None` on the interactive path.
        // That is the repair every privacy refusal in this crate now prints —
        // "re-run with `--provider versa_azure`" — and a refusal that names a
        // flag the command does not accept is worse than one that names none.
        #[command(flatten)]
        model_opts: ModelOptions,
    },

    /// Open the last project directory
    #[command(about = "Open the last project directory", visible_alias = "p")]
    Project {},

    /// List recent project directories
    #[command(about = "List recent project directories", visible_alias = "ps")]
    Projects,

    /// Execute commands from an instruction file
    #[command(about = "Execute commands from an instruction file or stdin")]
    Run {
        #[command(flatten)]
        input_opts: InputOptions,

        #[command(flatten)]
        identifier: Option<Identifier>,

        #[command(flatten)]
        run_behavior: RunBehavior,

        #[command(flatten)]
        session_opts: SessionOptions,

        #[command(flatten)]
        extension_opts: ExtensionOptions,

        #[command(flatten)]
        output_opts: OutputOptions,

        #[command(flatten)]
        model_opts: ModelOptions,
    },

    /// Workflow utilities for validation and deeplinking
    #[command(about = "Workflow utilities for validation and deeplinking")]
    Workflow {
        #[command(subcommand)]
        command: WorkflowCommand,
    },

    /// Inspect and update model/provider configuration
    #[command(about = "Inspect and update model/provider configuration")]
    Models {
        #[command(subcommand)]
        command: ModelsCommand,
    },

    /// Manage personal knowledge bases (ingest, lint, query)
    #[command(about = "Manage personal knowledge bases", visible_alias = "kb")]
    Knowledge {
        #[command(subcommand)]
        command: KnowledgeCommand,
    },

    /// Install and manage extensions (.brxt bundles)
    #[command(about = "Install and manage extensions", visible_alias = "ext")]
    Extension {
        #[command(subcommand)]
        command: ExtensionCommand,
    },

    /// Install and manage skills (.zip)
    #[command(about = "Install and manage skills")]
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },

    /// List, open, and serve Biorouter apps (built by Agent Drafter)
    #[command(
        about = "List, open, and serve Biorouter apps",
        long_about = "List, open, and serve the Biorouter apps built by Agent Drafter.\n\n\
                      `open`/`serve` reuse a `biorouterd` already listening on the configured\n\
                      port (BIOROUTER_PORT, default 3000) or start one for you, then open\n\
                      http://127.0.0.1:<port>/apps/<id>/ in your browser.\n\n\
                      In-terminal rendering of an app is out of scope; apps open in a real browser."
    )]
    Apps {
        #[command(subcommand)]
        command: AppsCommand,
    },

    /// Manage scheduled jobs
    #[command(about = "Manage scheduled jobs", visible_alias = "sched")]
    Schedule {
        #[command(subcommand)]
        command: SchedulerCommand,
    },

    /// Report token + cost usage per day or per model, with a month-to-date
    /// summary against the configured monthly budget.
    #[command(about = "Report token and cost usage (per day / per model, month-to-date)")]
    Usage {
        /// Start of the range, `YYYY-MM-DD` (local). Defaults to 30 days ago.
        #[arg(long, value_name = "YYYY-MM-DD")]
        from: Option<String>,

        /// End of the range, `YYYY-MM-DD` (local, inclusive). Defaults to today.
        #[arg(long, value_name = "YYYY-MM-DD")]
        to: Option<String>,

        /// Break the range down per model instead of per day.
        #[arg(long = "by-model", help = "Group usage by model instead of by day")]
        by_model: bool,

        /// Emit machine-readable JSON instead of a table.
        #[arg(long, help = "Print machine-readable JSON")]
        json: bool,
    },

    /// Check system prerequisites (git, uv, node, …) and the CLI install
    #[command(about = "Check system prerequisites and the CLI install")]
    Doctor {
        #[arg(
            long = "format",
            value_name = "FORMAT",
            default_value = "text",
            value_parser = clap::builder::PossibleValuesParser::new(["text", "json"])
        )]
        format: String,
        /// Skip the (networked) self-update check
        #[arg(long = "no-update")]
        no_update: bool,
        /// Hand a failing prerequisite to Biorouter: opens a session briefed with
        /// the dependency, this machine's environment and what to verify. Pass a
        /// name (`--fix uv`), or bare to take the first missing required one.
        #[arg(long = "fix", value_name = "DEPENDENCY", num_args = 0..=1, default_missing_value = "")]
        fix: Option<String>,
    },

    /// Install the `biorouter` command onto your PATH
    #[command(
        about = "Install the biorouter command onto your PATH",
        visible_alias = "install-cli"
    )]
    SetupPath {},

    /// Evaluate system configuration across a range of practical tasks
    #[command(about = "Evaluate system configuration across a range of practical tasks")]
    Bench {
        #[command(subcommand)]
        cmd: BenchCommand,
    },

    /// Run Biorouter and reach it from a browser
    ///
    /// `headless` is kept as an alias: it is the name the standalone binary
    /// this replaced was known by, so anyone following older instructions
    /// lands in the right place.
    #[command(
        about = "Run Biorouter and open it in a browser",
        visible_alias = "headless"
    )]
    Serve {
        /// Address to bind
        #[arg(
            long,
            default_value = "127.0.0.1",
            help = "Address to bind. Anything reachable from another machine requires a token."
        )]
        host: String,

        /// Port to listen on
        #[arg(
            short,
            long,
            default_value_t = crate::commands::serve::DEFAULT_PORT,
            help = "Port to listen on"
        )]
        port: u16,

        /// Use this access token instead of a freshly generated one
        #[arg(long, help = "Use this access token instead of generating one")]
        token: Option<String>,

        /// Serve without an access token
        #[arg(
            long,
            conflicts_with = "token",
            help = "Serve without an access token. Refused for a non-loopback bind."
        )]
        no_token: bool,

        /// Directory holding the built interface
        #[arg(
            long,
            help = "Directory holding the built web interface. Takes precedence over \
                    BIOROUTER_SERVE_UI; either must contain an index.html"
        )]
        web_dir: Option<std::path::PathBuf>,

        /// Open a browser once it is ready
        #[arg(long, help = "Open a browser once the server is ready")]
        open: bool,
    },

    /// Deprecated: use `biorouter serve`
    ///
    /// This is an inherited, hand-written chat page -- not the Biorouter
    /// interface. It defaults to port 3000, which collides with the daemon, and
    /// it shares one agent across every chat. `serve` supersedes it on every
    /// axis, so this is hidden from help and forwards a notice; it still runs,
    /// so anyone following older instructions is told where to go rather than
    /// hitting an unknown-command error.
    #[command(hide = true, about = "Deprecated: use `biorouter serve` instead")]
    Web {
        /// Port to run the web server on
        #[arg(
            short,
            long,
            default_value = "3000",
            help = "Port to run the web server on"
        )]
        port: u16,

        /// Host to bind the web server to
        #[arg(
            long,
            default_value = "127.0.0.1",
            help = "Host to bind the web server to"
        )]
        host: String,

        /// Open browser automatically
        #[arg(long, help = "Open browser automatically when server starts")]
        open: bool,

        /// Authentication token for both Basic Auth (password) and Bearer token
        #[arg(long, help = "Authentication token to secure the web interface")]
        auth_token: Option<String>,

        /// Allow running without authentication when exposed on the network (unsafe)
        #[arg(
            long,
            help = "Skip auth requirement when exposed on the network (unsafe)"
        )]
        no_auth: bool,
    },

    /// Terminal-integrated session (one session per terminal)
    #[command(
        about = "Terminal-integrated Biorouter session",
        long_about = "Runs a Biorouter session tied to your terminal window.\n\
                      Each terminal maintains its own persistent session that resumes automatically.\n\n\
                      Setup:\n  \
                        eval \"$(biorouter term init zsh)\"  # Add to ~/.zshrc\n\n\
                      Usage:\n  \
                        biorouter term run \"list files in this directory\"\n  \
                        @biorouter \"create a python script\"  # using alias\n  \
                        @g \"quick question\"  # short alias"
    )]
    Term {
        #[command(subcommand)]
        command: TermCommand,
    },
    /// Generate completions for various shells
    #[command(about = "Generate the autocompletion script for the specified shell")]
    Completion {
        #[arg(value_enum)]
        shell: ClapShell,

        #[arg(
            long,
            default_value = "biorouter",
            help = "Provide a custom binary name"
        )]
        bin_name: String,
    },
}

#[derive(Subcommand)]
enum TermCommand {
    /// Print shell initialization script
    #[command(
        about = "Print shell initialization script",
        long_about = "Prints shell configuration to set up terminal-integrated sessions.\n\
                      Each terminal gets a persistent biorouter session that automatically resumes.\n\n\
                      Setup:\n  \
                        echo 'eval \"$(biorouter term init zsh)\"' >> ~/.zshrc\n  \
                        source ~/.zshrc\n\n\
                      With --default (anything typed that isn't a command goes to biorouter):\n  \
                        echo 'eval \"$(biorouter term init zsh --default)\"' >> ~/.zshrc"
    )]
    Init {
        /// Shell type (bash, zsh, fish, powershell)
        #[arg(value_enum)]
        shell: Shell,

        #[arg(short, long, help = "Name for the terminal session")]
        name: Option<String>,

        /// Make Biorouter the default handler for unknown commands
        #[arg(
            long = "default",
            help = "Make Biorouter the default handler for unknown commands",
            long_help = "When enabled, anything you type that is not a valid command will be sent to Biorouter. Only supported for zsh and bash."
        )]
        default: bool,
    },

    /// Log a shell command (called by shell hook)
    #[command(about = "Log a shell command to the session", hide = true)]
    Log {
        /// The command that was executed
        command: String,
    },

    /// Run a prompt in the terminal session
    #[command(
        about = "Run a prompt in the terminal session",
        long_about = "Run a prompt in the terminal-integrated session.\n\n\
                      Examples:\n  \
                        biorouter term run list files in this directory\n  \
                        @biorouter list files  # using alias\n  \
                        @g why did that fail  # short alias"
    )]
    Run {
        /// The prompt to send to Biorouter (multiple words allowed without quotes)
        #[arg(required = true, num_args = 1..)]
        prompt: Vec<String>,
    },

    /// Print session info for prompt integration
    #[command(
        about = "Print session info for prompt integration",
        long_about = "Prints compact session info (token usage, model) for shell prompt integration.\n\
                      Example output: ●○○○○ sonnet"
    )]
    Info,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum CliProviderVariant {
    OpenAi,
    Databricks,
    Ollama,
}

#[derive(Debug)]
pub struct InputConfig {
    pub contents: Option<String>,
    pub additional_system_prompt: Option<String>,
}

fn get_command_name(command: &Option<Command>) -> &'static str {
    match command {
        Some(Command::Configure {}) => "configure",
        Some(Command::Info { .. }) => "info",
        Some(Command::Mcp { .. }) => "mcp",
        Some(Command::Acp { .. }) => "acp",
        Some(Command::Session { .. }) => "session",
        Some(Command::Project {}) => "project",
        Some(Command::Projects) => "projects",
        Some(Command::Run { .. }) => "run",
        Some(Command::Schedule { .. }) => "schedule",
        Some(Command::Usage { .. }) => "usage",
        Some(Command::Models { .. }) => "models",
        Some(Command::Knowledge { .. }) => "knowledge",
        Some(Command::Extension { .. }) => "extension",
        Some(Command::Skill { .. }) => "skill",
        Some(Command::Apps { .. }) => "apps",
        Some(Command::Doctor { .. }) => "doctor",
        Some(Command::SetupPath { .. }) => "setup-path",
        Some(Command::Bench { .. }) => "bench",
        Some(Command::Workflow { .. }) => "workflow",
        Some(Command::Serve { .. }) => "serve",
        Some(Command::Web { .. }) => "web",
        Some(Command::Term { .. }) => "term",
        Some(Command::Completion { .. }) => "completion",
        None => "default_session",
    }
}

async fn handle_mcp_command(server: McpCommand) -> Result<()> {
    let name = server.name();
    crate::logging::setup_logging(Some(&format!("mcp-{name}")), None)?;
    match server {
        McpCommand::AutoVisualiser => serve(AutoVisualiserRouter::new()).await?,
        McpCommand::ComputerController => serve(ComputerControllerServer::new()).await?,
        McpCommand::Memory => serve(MemoryServer::new()).await?,
        McpCommand::Developer => serve(DeveloperServer::new()).await?,
    }
    Ok(())
}

async fn handle_session_subcommand(command: SessionCommand) -> Result<()> {
    match command {
        SessionCommand::List {
            format,
            ascending,
            working_dir,
            limit,
            subagents,
        } => {
            handle_session_list(format, ascending, working_dir, limit, subagents).await?;
        }
        SessionCommand::Remove { identifier, regex } => {
            let (session_id, name) = if let Some(id) = identifier {
                (id.session_id, id.name)
            } else {
                (None, None)
            };
            handle_session_remove(session_id, name, regex).await?;
        }
        SessionCommand::Export {
            identifier,
            output,
            format,
        } => {
            let session_manager = SessionManager::instance();
            let session_identifier = if let Some(id) = identifier {
                lookup_session_id(id).await?
            } else {
                match crate::commands::session::prompt_interactive_session_selection(
                    &session_manager,
                )
                .await
                {
                    Ok(id) => id,
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        return Ok(());
                    }
                }
            };
            crate::commands::session::handle_session_export(session_identifier, output, format)
                .await?;
        }
        SessionCommand::Watch { session_id, follow } => {
            crate::commands::session_watch::handle_session_watch(&session_id, follow).await?;
        }
        SessionCommand::Send {
            session_id,
            text,
            no_wait,
            user_action_key_stdin,
        } => {
            crate::commands::session_watch::handle_session_send(
                &session_id,
                &text,
                !no_wait,
                user_action_key_stdin,
            )
            .await?;
        }
        SessionCommand::Attach {
            session_id,
            name,
            of,
            read_only,
            user_action_key_stdin,
        } => {
            crate::commands::session_watch::handle_session_attach(
                session_id,
                name,
                of,
                read_only,
                user_action_key_stdin,
            )
            .await?;
        }
        SessionCommand::Cancel {
            session_id,
            user_action_key_stdin,
        } => {
            crate::commands::session_watch::handle_session_cancel(
                &session_id,
                user_action_key_stdin,
            )
            .await?;
        }
        SessionCommand::Diagnostics { identifier, output } => {
            let session_manager = SessionManager::instance();
            let session_id = if let Some(id) = identifier {
                lookup_session_id(id).await?
            } else {
                match crate::commands::session::prompt_interactive_session_selection(
                    &session_manager,
                )
                .await
                {
                    Ok(id) => id,
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        return Ok(());
                    }
                }
            };
            crate::commands::session::handle_diagnostics(&session_id, output).await?;
        }
        SessionCommand::Rename {
            identifier,
            new_name,
        } => {
            let session_manager = SessionManager::instance();
            let session_id = if let Some(id) = identifier {
                lookup_session_id(id).await?
            } else {
                match crate::commands::session::prompt_interactive_session_selection(
                    &session_manager,
                )
                .await
                {
                    Ok(id) => id,
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        return Ok(());
                    }
                }
            };
            crate::commands::session::handle_session_rename(&session_id, new_name).await?;
        }
        SessionCommand::Diverge {
            identifier,
            branch_name,
        } => {
            let session_manager = SessionManager::instance();
            let session_id = if let Some(id) = identifier {
                lookup_session_id(id).await?
            } else {
                match crate::commands::session::prompt_interactive_session_selection(
                    &session_manager,
                )
                .await
                {
                    Ok(id) => id,
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        return Ok(());
                    }
                }
            };
            crate::commands::session::handle_session_diverge(&session_id, branch_name).await?;
        }
        SessionCommand::Declassify { session_id } => {
            // No `lookup_session_id`, no interactive picker: both go through
            // `list_sessions*`, which is exactly the filter this subcommand
            // exists to route around.
            crate::commands::session::declassify_command(&session_id).await?;
        }
    }
    Ok(())
}

/// `biorouter session` — both of its forms, dispatched in one place.
///
/// ⚠ **Extracted from `cli()` rather than inlined, and the reason is mechanical.**
/// Task 31 (issue #56) gave the interactive form a seventh field, `model_opts`,
/// which pushed the call past one line and `cli()` past clippy's 100-line
/// ceiling (`too_many_lines`, 103/100 — a NEW violation against
/// `scripts/clippy-lint.sh`'s baseline, so CI fails on it). The honest fix is to
/// move the arm's body out, not to `#[allow]` the lint on a function that has
/// been growing an arm per feature for years.
async fn handle_session_command(
    command: Option<SessionCommand>,
    identifier: Option<Identifier>,
    resume: bool,
    history: bool,
    session_opts: SessionOptions,
    extension_opts: ExtensionOptions,
    model_opts: ModelOptions,
) -> Result<()> {
    match command {
        Some(cmd) => handle_session_subcommand(cmd).await,
        None => {
            handle_interactive_session(
                identifier,
                resume,
                history,
                session_opts,
                extension_opts,
                model_opts,
            )
            .await
        }
    }
}

async fn handle_interactive_session(
    identifier: Option<Identifier>,
    resume: bool,
    history: bool,
    session_opts: SessionOptions,
    extension_opts: ExtensionOptions,
    model_opts: ModelOptions,
) -> Result<()> {
    let session_start = std::time::Instant::now();
    let session_type = if resume { "resumed" } else { "new" };

    tracing::info!(
        counter.biorouter.session_starts = 1,
        session_type,
        interactive = true,
        "Session started"
    );

    if let Some(Identifier {
        session_id: Some(_),
        ..
    }) = &identifier
    {
        if !resume {
            eprintln!("Error: --session-id can only be used with --resume flag");
            std::process::exit(1);
        }
    }

    let session_id = get_or_create_session_id(
        identifier,
        resume,
        false,
        model_opts.provider.as_deref(),
        model_opts.model.as_deref(),
    )
    .await?;

    let mut session: crate::CliSession = build_session(SessionBuilderConfig {
        session_id,
        resume,
        no_session: false,
        extensions: extension_opts.extensions,
        streamable_http_extensions: extension_opts.streamable_http_extensions,
        builtins: extension_opts.builtins,
        workflow: None,
        additional_system_prompt: None,
        provider: model_opts.provider,
        model: model_opts.model,
        debug: session_opts.debug,
        max_tool_repetitions: session_opts.max_tool_repetitions,
        max_turns: session_opts.max_turns,
        scheduled_job_id: None,
        interactive: true,
        quiet: false,
        output_format: "text".to_string(),
    })
    .await;

    if resume && history {
        session.render_message_history();
    }

    let result = session.interactive(None).await;
    log_session_completion(&session, session_start, session_type, result.is_ok()).await;
    result
}

async fn log_session_completion(
    session: &crate::CliSession,
    session_start: std::time::Instant,
    session_type: &str,
    success: bool,
) {
    let session_duration = session_start.elapsed();
    let exit_type = if success { "normal" } else { "error" };

    let (total_tokens, message_count) = session
        .get_session()
        .await
        .map(|m| (m.total_tokens.unwrap_or(0), m.message_count))
        .unwrap_or((0, 0));

    tracing::info!(
        counter.biorouter.session_completions = 1,
        session_type,
        exit_type,
        duration_ms = session_duration.as_millis() as u64,
        total_tokens,
        message_count,
        "Session completed"
    );

    tracing::info!(
        counter.biorouter.session_duration_ms = session_duration.as_millis() as u64,
        session_type,
        "Session duration"
    );

    if total_tokens > 0 {
        tracing::info!(
            counter.biorouter.session_tokens = total_tokens,
            session_type,
            "Session tokens"
        );
    }
}

fn parse_run_input(
    input_opts: &InputOptions,
    quiet: bool,
) -> Result<Option<(InputConfig, Option<Workflow>)>> {
    match (
        &input_opts.instructions,
        &input_opts.input_text,
        &input_opts.workflow,
    ) {
        (Some(file), _, _) if file == "-" => {
            let mut contents = String::new();
            std::io::stdin()
                .read_to_string(&mut contents)
                .expect("Failed to read from stdin");
            Ok(Some((
                InputConfig {
                    contents: Some(contents),
                    additional_system_prompt: input_opts.system.clone(),
                },
                None,
            )))
        }
        (Some(file), _, _) => {
            let contents = std::fs::read_to_string(file).unwrap_or_else(|err| {
                eprintln!(
                    "Instruction file not found. Did you mean to use biorouter run --text?\n{}",
                    err
                );
                std::process::exit(1);
            });
            Ok(Some((
                InputConfig {
                    contents: Some(contents),
                    additional_system_prompt: None,
                },
                None,
            )))
        }
        (_, Some(text), _) => Ok(Some((
            InputConfig {
                contents: Some(text.clone()),
                additional_system_prompt: input_opts.system.clone(),
            },
            None,
        ))),
        (_, _, Some(workflow_name)) => {
            let workflow_display_name = std::path::Path::new(workflow_name)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(workflow_name);

            let workflow_version =
                crate::workflows::search_workflow::load_workflow_file(workflow_name)
                    .ok()
                    .and_then(|rf| {
                        biorouter::workflow::template_workflow::parse_workflow_content(
                            &rf.content,
                            Some(rf.parent_dir.display().to_string()),
                        )
                        .ok()
                        .map(|(r, _)| r.version)
                    })
                    .unwrap_or_else(|| "unknown".to_string());

            if input_opts.explain {
                explain_workflow(workflow_name, input_opts.params.clone())?;
                return Ok(None);
            }
            if input_opts.render_workflow {
                if let Err(err) = render_workflow_as_yaml(workflow_name, input_opts.params.clone())
                {
                    eprintln!("{}: {}", console::style("Error").red().bold(), err);
                    std::process::exit(1);
                }
                return Ok(None);
            }

            tracing::info!(
                counter.biorouter.workflow_runs = 1,
                workflow_name = %workflow_display_name,
                workflow_version = %workflow_version,
                session_type = "workflow",
                interface = "cli",
                "Workflow execution started"
            );

            let (input_config, workflow) = extract_workflow_info_from_cli(
                workflow_name.clone(),
                input_opts.params.clone(),
                input_opts.additional_sub_workflows.clone(),
                quiet,
            )?;
            Ok(Some((input_config, Some(workflow))))
        }
        (None, None, None) => {
            eprintln!("Error: Must provide either --instructions (-i), --text (-t), or --workflow. Use -i - for stdin.");
            std::process::exit(1);
        }
    }
}

async fn handle_run_command(
    input_opts: InputOptions,
    identifier: Option<Identifier>,
    run_behavior: RunBehavior,
    session_opts: SessionOptions,
    extension_opts: ExtensionOptions,
    output_opts: OutputOptions,
    model_opts: ModelOptions,
) -> Result<()> {
    let parsed = parse_run_input(&input_opts, output_opts.quiet)?;

    let Some((input_config, workflow)) = parsed else {
        return Ok(());
    };

    if let Some(Identifier {
        session_id: Some(_),
        ..
    }) = &identifier
    {
        if !run_behavior.resume {
            eprintln!("Error: --session-id can only be used with --resume flag");
            std::process::exit(1);
        }
    }

    // The workflow's pins count as configuration: a workflow that names its own
    // provider is a run the user CAN start without a global default.
    let workflow_settings = workflow.as_ref().and_then(|w| w.settings.as_ref());
    let provider_for_precheck = model_opts
        .provider
        .clone()
        .or_else(|| workflow_settings.and_then(|s| s.biorouter_provider.clone()));
    let model_for_precheck = model_opts
        .model
        .clone()
        .or_else(|| workflow_settings.and_then(|s| s.biorouter_model.clone()));
    let session_id = get_or_create_session_id(
        identifier,
        run_behavior.resume,
        run_behavior.no_session,
        provider_for_precheck.as_deref(),
        model_for_precheck.as_deref(),
    )
    .await?;

    let mut session = build_session(SessionBuilderConfig {
        session_id,
        resume: run_behavior.resume,
        no_session: run_behavior.no_session,
        extensions: extension_opts.extensions,
        streamable_http_extensions: extension_opts.streamable_http_extensions,
        builtins: extension_opts.builtins,
        workflow: workflow.clone(),
        additional_system_prompt: input_config.additional_system_prompt,
        provider: model_opts.provider,
        model: model_opts.model,
        debug: session_opts.debug,
        max_tool_repetitions: session_opts.max_tool_repetitions,
        max_turns: session_opts.max_turns,
        scheduled_job_id: run_behavior.scheduled_job_id,
        interactive: run_behavior.interactive,
        quiet: output_opts.quiet,
        output_format: output_opts.output_format,
    })
    .await;

    if run_behavior.interactive {
        session.interactive(input_config.contents).await
    } else if let Some(contents) = input_config.contents {
        let session_start = std::time::Instant::now();
        let session_type = if workflow.is_some() {
            "workflow"
        } else {
            "run"
        };

        tracing::info!(
            counter.biorouter.session_starts = 1,
            session_type,
            interactive = false,
            "Headless session started"
        );

        let result = session.headless(contents).await;
        log_session_completion(&session, session_start, session_type, result.is_ok()).await;
        result
    } else {
        Err(anyhow::anyhow!(
            "no text provided for prompt in headless mode"
        ))
    }
}

async fn handle_schedule_command(command: SchedulerCommand) -> Result<()> {
    match command {
        SchedulerCommand::Add {
            schedule_id,
            cron,
            workflow_source,
        } => handle_schedule_add(schedule_id, cron, workflow_source).await,
        SchedulerCommand::List {} => handle_schedule_list().await,
        SchedulerCommand::Remove { schedule_id } => handle_schedule_remove(schedule_id).await,
        SchedulerCommand::Sessions { schedule_id, limit } => {
            handle_schedule_sessions(schedule_id, limit).await
        }
        SchedulerCommand::RunNow { schedule_id } => handle_schedule_run_now(schedule_id).await,
        SchedulerCommand::ServicesStatus {} => handle_schedule_services_status().await,
        SchedulerCommand::ServicesStop {} => handle_schedule_services_stop().await,
        SchedulerCommand::CronHelp {} => handle_schedule_cron_help().await,
    }
}

async fn handle_bench_command(cmd: BenchCommand) -> Result<()> {
    match cmd {
        BenchCommand::Selectors { config } => BenchRunner::list_selectors(config)?,
        BenchCommand::InitConfig { name } => {
            let mut config = BenchRunConfig::default();
            let cwd = std::env::current_dir()?;
            config.output_dir = Some(cwd);
            config.save(name);
        }
        BenchCommand::Run { config } => BenchRunner::new(config)?.run()?,
        BenchCommand::EvalModel { config } => ModelRunner::from(config)?.run()?,
        BenchCommand::ExecEval { config } => EvalRunner::from(config)?.run(agent_generator).await?,
        BenchCommand::GenerateLeaderboard { benchmark_dir } => {
            MetricAggregator::generate_csv_from_benchmark_dir(&benchmark_dir)?
        }
    }
    Ok(())
}

fn handle_workflow_subcommand(command: WorkflowCommand) -> Result<()> {
    match command {
        WorkflowCommand::Install { path } => crate::commands::workflow::handle_install(&path),
        WorkflowCommand::Validate { workflow_name } => handle_validate(&workflow_name),
        WorkflowCommand::Deeplink {
            workflow_name,
            params,
        } => {
            handle_deeplink(&workflow_name, &params)?;
            Ok(())
        }
        WorkflowCommand::Open {
            workflow_name,
            params,
        } => handle_open(&workflow_name, &params),
        WorkflowCommand::List { format, verbose } => handle_list(&format, verbose),
    }
}

async fn handle_models_subcommand(command: ModelsCommand) -> Result<()> {
    match command {
        ModelsCommand::Current { format } => handle_models_current(&format).await,
        ModelsCommand::Providers { format } => handle_models_providers(&format).await,
        ModelsCommand::List { provider, format } => handle_models_list(provider, &format).await,
        ModelsCommand::Set { provider, model } => handle_models_set(provider, model).await,
        ModelsCommand::Local { command } => match command {
            LocalModelCommand::List { format } => {
                crate::commands::models::handle_models_local_list(&format).await
            }
            LocalModelCommand::Pull { model } => {
                crate::commands::models::handle_models_local_pull(model).await
            }
            LocalModelCommand::Rm { model } => {
                crate::commands::models::handle_models_local_rm(model).await
            }
        },
    }
}

async fn handle_knowledge_subcommand(command: KnowledgeCommand) -> Result<()> {
    use crate::commands::knowledge;
    // A CLI-only process never reaches the agent runtime's startup migration,
    // so this boundary must finish the legacy purge before any knowledge
    // command can observe the store. The remaining built-in assets stay
    // best-effort; the Daily Meditation schedule is registered by the agent
    // runtime where a scheduler is available.
    //
    // `list` is exempt, and deliberately: it is the command a user runs to find
    // out what the store holds, and answering that question by irreversibly
    // deleting part of it is indefensible. They see the legacy bases named and
    // choose. `install_assets` still registers the built-in Soul, which it does
    // without purging.
    // Resolved once here, at the top of the command, and passed down: the
    // seeders take their root explicitly so nothing re-reads the process
    // environment part-way through. For a real CLI invocation this is exactly
    // the ambient root, as it always was.
    let config_dir = biorouter::config::paths::Paths::config_dir();
    if !matches!(command, KnowledgeCommand::List { .. }) {
        biorouter::knowledge::soul::ensure_soul_kb(&config_dir)?;
    }
    biorouter::knowledge::soul::install_assets(&config_dir);
    match command {
        KnowledgeCommand::List { format } => knowledge::handle_list(&format).await,
        KnowledgeCommand::Active {
            set,
            clear,
            inherit,
            session,
        } => knowledge::handle_active(set, clear, inherit, session).await,
        KnowledgeCommand::Create { id, name, color } => {
            knowledge::handle_create(id, name, color).await
        }
        KnowledgeCommand::Ingest {
            kb,
            url,
            file,
            text,
            focus,
            provider,
            model,
        } => knowledge::handle_ingest(kb, url, file, text, focus, provider, model).await,
        KnowledgeCommand::IngestConversation {
            kb,
            session,
            new_kb,
            focus,
            provider,
            model,
        } => {
            knowledge::handle_ingest_conversation(kb, session, new_kb, focus, provider, model).await
        }
        KnowledgeCommand::Lint {
            kb,
            fix,
            provider,
            model,
        } => knowledge::handle_lint(kb, fix, provider, model).await,
        KnowledgeCommand::Hide { id } => knowledge::handle_hide(id).await,
        KnowledgeCommand::Unhide { id } => knowledge::handle_unhide(id).await,
        KnowledgeCommand::Query {
            question,
            kb,
            save,
            provider,
            model,
        } => knowledge::handle_query(question, kb, save, provider, model).await,
    }
}

async fn handle_extension_subcommand(command: ExtensionCommand) -> Result<()> {
    use crate::commands::extension;
    match command {
        ExtensionCommand::Install {
            path,
            env,
            secret,
            secret_stdin,
            no_enable,
        } => extension::handle_install(path, env, secret, secret_stdin, no_enable).await,
        ExtensionCommand::List { format } => extension::handle_list(&format).await,
        ExtensionCommand::Configure { name } => extension::handle_configure(name).await,
        ExtensionCommand::Remove { name, purge } => extension::handle_remove(name, purge).await,
    }
}

async fn handle_skill_subcommand(command: SkillCommand) -> Result<()> {
    use crate::commands::skill;
    match command {
        SkillCommand::Install {
            source,
            force,
            install_as,
        } => skill::handle_install(source, force, install_as).await,
        SkillCommand::List {} => skill::handle_list().await,
        SkillCommand::Enable { name } => skill::handle_enable(name).await,
        SkillCommand::Disable { name } => skill::handle_disable(name).await,
        SkillCommand::Remove { slug } => skill::handle_remove(slug).await,
    }
}

async fn handle_apps_subcommand(command: AppsCommand) -> Result<()> {
    match command {
        AppsCommand::List { json } => handle_apps_list(json).await,
        AppsCommand::Open { id } => handle_apps_open(id).await,
        AppsCommand::Serve { id } => handle_apps_serve(id).await,
    }
}

async fn handle_term_subcommand(command: TermCommand) -> Result<()> {
    match command {
        TermCommand::Init {
            shell,
            name,
            default,
        } => handle_term_init(shell, name, default).await,
        TermCommand::Log { command } => handle_term_log(command).await,
        TermCommand::Run { prompt } => handle_term_run(prompt).await,
        TermCommand::Info => handle_term_info().await,
    }
}

/// `biorouter doctor --fix [DEP]` — the terminal half of the desktop's
/// "Debug with Biorouter" button.
///
/// The briefing is built by `biorouter::system::debug_prompt`, the same function
/// the rest of the app uses, and handed to a normal interactive session as its
/// opening message. Nothing about the session is special: it has the shell, and
/// the user can steer it. What it saves is the twenty questions the agent would
/// otherwise have to ask about which command failed and on what machine.
async fn handle_doctor_fix(dep: String) -> Result<()> {
    let deps = biorouter::system::check_all();

    let chosen = if dep.is_empty() {
        // Bare `--fix`: the first missing required prerequisite, then any missing
        // optional one. Nothing missing is a success, not an error.
        deps.iter()
            .find(|d| d.required && !d.installed)
            .or_else(|| deps.iter().find(|d| !d.installed))
            .cloned()
    } else {
        match biorouter::system::status_of(&dep) {
            Some(status) => Some(status),
            // Not one of Biorouter's tracked prerequisites — `docker`, `jq`,
            // `shellcheck` and friends are all things a setup script can die on,
            // and refusing to help with them would make the hint those scripts
            // print a lie. Synthesize a status so the session still gets a
            // briefing that names the tool and this machine.
            None => {
                let known: Vec<&str> = deps.iter().map(|d| d.name.as_str()).collect();
                println!(
                    "`{dep}` is not one of Biorouter's tracked prerequisites ({}), \
                     so there is no install command on file for it. Starting a session anyway.",
                    known.join(", ")
                );
                Some(biorouter::system::DependencyStatus {
                    name: dep.clone(),
                    display_name: dep.clone(),
                    installed: false,
                    version: None,
                    required: false,
                    purpose: "Required by a Biorouter setup or build script".to_string(),
                    doc_url: String::new(),
                    install_command: None,
                    requires_sudo: false,
                    download_url: None,
                })
            }
        }
    };

    let Some(status) = chosen else {
        println!("All prerequisites are present — nothing to fix.");
        return Ok(());
    };

    if status.installed {
        println!(
            concat!(
                "{} is already detected ({}). Starting a session anyway so you can ",
                "describe what is going wrong."
            ),
            status.display_name,
            status.version.as_deref().unwrap_or("version unknown")
        );
    }

    let prompt = biorouter::system::debug_prompt(&biorouter::system::DependencyFailure {
        status: &status,
        output: None,
        error: None,
    });

    if !Config::global().exists() {
        anyhow::bail!(concat!(
            "Biorouter is not configured yet, so there is no model to debug with. ",
            "Run `biorouter configure` first."
        ));
    }

    let session_id = get_or_create_session_id(None, false, false, None, None).await?;
    let mut session = build_session(SessionBuilderConfig {
        session_id,
        resume: false,
        no_session: false,
        extensions: Vec::new(),
        streamable_http_extensions: Vec::new(),
        builtins: Vec::new(),
        workflow: None,
        additional_system_prompt: None,
        provider: None,
        model: None,
        debug: false,
        max_tool_repetitions: None,
        max_turns: None,
        scheduled_job_id: None,
        interactive: true,
        quiet: false,
        output_format: "text".to_string(),
    })
    .await;
    session.interactive(Some(prompt)).await
}

async fn handle_default_session() -> Result<()> {
    if !Config::global().exists() {
        return handle_configure().await;
    }

    let session_id = get_or_create_session_id(None, false, false, None, None).await?;

    let mut session = build_session(SessionBuilderConfig {
        session_id,
        resume: false,
        no_session: false,
        extensions: Vec::new(),
        streamable_http_extensions: Vec::new(),
        builtins: Vec::new(),
        workflow: None,
        additional_system_prompt: None,
        provider: None,
        model: None,
        debug: false,
        max_tool_repetitions: None,
        max_turns: None,
        scheduled_job_id: None,
        interactive: true,
        quiet: false,
        output_format: "text".to_string(),
    })
    .await;
    session.interactive(None).await
}

pub async fn cli() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Err(e) = crate::project_tracker::update_project_tracker(None, None) {
        warn!("Warning: Failed to update project tracker: {}", e);
    }

    let command_name = get_command_name(&cli.command);
    tracing::info!(
        counter.biorouter.cli_commands = 1,
        command = command_name,
        "CLI command executed"
    );

    dispatch(cli.command).await
}

/// Run the named command.
///
/// Split out of [`cli`] so that adding a verb grows a function that is nothing
/// but arms. `clippy::too_many_lines` is enforced against a baseline here, and
/// a dispatch table is exactly the shape that limit should not be spent on --
/// while the setup above it is the part worth keeping short.
///
/// The match stays exhaustive with no wildcard arm: a new `Command` variant
/// must fail to compile in both this function and `get_command_name`, so it
/// cannot ship silently unreachable.
async fn dispatch(command: Option<Command>) -> anyhow::Result<()> {
    match command {
        Some(Command::Completion { shell, bin_name }) => {
            let mut cmd = Cli::command();
            generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
            Ok(())
        }
        Some(Command::Configure {}) => handle_configure().await,
        Some(Command::Info { verbose }) => handle_info(verbose),
        Some(Command::Mcp { server }) => handle_mcp_command(server).await,
        Some(Command::Acp { builtins, ws }) => match ws {
            Some(addr) => biorouter_acp::server::run_ws(builtins, addr).await,
            None => biorouter_acp::server::run(builtins).await,
        },
        Some(Command::Session {
            command,
            identifier,
            resume,
            history,
            session_opts,
            extension_opts,
            model_opts,
        }) => {
            handle_session_command(
                command,
                identifier,
                resume,
                history,
                session_opts,
                extension_opts,
                model_opts,
            )
            .await
        }
        // Both already return `anyhow::Result<()>`, so `?` followed by `Ok(())`
        // was a six-line restatement of the value the call hands back.
        Some(Command::Project {}) => handle_project_default(),
        Some(Command::Projects) => handle_projects_interactive(),
        Some(Command::Run {
            input_opts,
            identifier,
            run_behavior,
            session_opts,
            extension_opts,
            output_opts,
            model_opts,
        }) => {
            handle_run_command(
                input_opts,
                identifier,
                run_behavior,
                session_opts,
                extension_opts,
                output_opts,
                model_opts,
            )
            .await
        }
        Some(Command::Schedule { command }) => handle_schedule_command(command).await,
        Some(Command::Usage {
            from,
            to,
            by_model,
            json,
        }) => crate::commands::usage::handle_usage(from, to, by_model, json).await,
        Some(Command::Doctor {
            format,
            no_update,
            fix,
        }) => match fix {
            Some(dep) => handle_doctor_fix(dep).await,
            None => crate::commands::doctor::handle_doctor(&format, !no_update).await,
        },
        Some(Command::SetupPath {}) => crate::commands::doctor::handle_setup_path().await,
        Some(Command::Bench { cmd }) => handle_bench_command(cmd).await,
        Some(Command::Workflow { command }) => handle_workflow_subcommand(command),
        Some(Command::Models { command }) => handle_models_subcommand(command).await,
        Some(Command::Knowledge { command }) => handle_knowledge_subcommand(command).await,
        Some(Command::Extension { command }) => handle_extension_subcommand(command).await,
        Some(Command::Skill { command }) => handle_skill_subcommand(command).await,
        Some(Command::Apps { command }) => handle_apps_subcommand(command).await,
        Some(Command::Serve {
            host,
            port,
            token,
            no_token,
            web_dir,
            open,
        }) => {
            crate::commands::serve::handle_serve(host, port, token, no_token, web_dir, open).await
        }
        Some(Command::Web {
            port,
            host,
            open,
            auth_token,
            no_auth,
        }) => crate::commands::web::handle_web(port, host, open, auth_token, no_auth).await,
        Some(Command::Term { command }) => handle_term_subcommand(command).await,
        None => handle_default_session().await,
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[tokio::test]
    #[serial_test::serial]
    async fn cli_knowledge_startup_purges_legacy_bases_and_preserves_current_profiles() {
        fn make_legacy(
            svc: &biorouter::knowledge::service::KnowledgeService,
            id: &str,
            name: &str,
        ) {
            svc.create_base(id, name, None).unwrap();
            let root = biorouter::knowledge::paths::kb_root(svc.root(), id);
            let mut manifest = biorouter::knowledge::manifest::load(&root).unwrap();
            manifest.schema_version = biorouter::knowledge::types::AUTOMATIC_SCHEMA_CEILING;
            biorouter::knowledge::manifest::save(&root, &manifest).unwrap();
        }

        let tmp = tempfile::tempdir().unwrap();
        let _env = env_lock::lock_env([
            (
                "BIOROUTER_PATH_ROOT",
                Some(tmp.path().to_string_lossy().into_owned()),
            ),
            ("BIOROUTER_DISABLE_KEYRING", Some("1".to_string())),
        ]);
        let svc = biorouter::knowledge::service::KnowledgeService::new_default().unwrap();
        make_legacy(&svc, "soul", "Soul");
        make_legacy(&svc, "legacy-notes", "Legacy notes");
        svc.create_base_in(
            "current-okf",
            "Current OKF",
            None,
            biorouter::knowledge::types::KbFormat::Okf,
        )
        .unwrap();
        svc.create_base_in(
            "current-biookf",
            "Current BioOKF",
            None,
            biorouter::knowledge::types::KbFormat::Biookf,
        )
        .unwrap();

        let okf_marker = svc.root().join("current-okf").join("preserved.txt");
        let biookf_marker = svc.root().join("current-biookf").join("preserved.txt");
        std::fs::write(&okf_marker, "okf survives").unwrap();
        std::fs::write(&biookf_marker, "biookf survives").unwrap();
        let legacy_soul_marker = svc.root().join("soul").join("legacy.txt");
        std::fs::write(&legacy_soul_marker, "must be purged").unwrap();

        // Driven through a mutating command, not `list`: the purge is what this
        // test is about, and `list` no longer triggers it.
        for i in 0..2 {
            handle_knowledge_subcommand(KnowledgeCommand::Create {
                id: format!("purge-driver-{i}"),
                name: Some(format!("Purge driver {i}")),
                color: None,
            })
            .await
            .unwrap();
        }

        assert!(svc.get_base("legacy-notes").is_err());
        assert!(!legacy_soul_marker.exists());
        assert_eq!(
            svc.get_base("soul").unwrap().profile(),
            Some(biorouter::knowledge::types::KbFormat::Okf)
        );
        assert_eq!(
            svc.get_base("current-okf").unwrap().profile(),
            Some(biorouter::knowledge::types::KbFormat::Okf)
        );
        assert_eq!(
            svc.get_base("current-biookf").unwrap().profile(),
            Some(biorouter::knowledge::types::KbFormat::Biookf)
        );
        assert_eq!(std::fs::read_to_string(okf_marker).unwrap(), "okf survives");
        assert_eq!(
            std::fs::read_to_string(biookf_marker).unwrap(),
            "biookf survives"
        );
    }

    /// `knowledge list` enumerates the store; it must not edit it.
    ///
    /// The purge is irreversible, and `list` is precisely the command someone
    /// runs to find out what they have before deciding. A listing that deletes
    /// the thing it is reporting leaves the user no moment at which to choose.
    #[tokio::test]
    async fn cli_knowledge_list_reports_legacy_bases_without_purging_them() {
        fn make_legacy(
            svc: &biorouter::knowledge::service::KnowledgeService,
            id: &str,
            name: &str,
        ) {
            svc.create_base(id, name, None).unwrap();
            let root = biorouter::knowledge::paths::kb_root(svc.root(), id);
            let mut manifest = biorouter::knowledge::manifest::load(&root).unwrap();
            manifest.schema_version = biorouter::knowledge::types::AUTOMATIC_SCHEMA_CEILING;
            biorouter::knowledge::manifest::save(&root, &manifest).unwrap();
        }

        let tmp = tempfile::tempdir().unwrap();
        let _env = env_lock::lock_env([
            (
                "BIOROUTER_PATH_ROOT",
                Some(tmp.path().to_string_lossy().into_owned()),
            ),
            ("BIOROUTER_DISABLE_KEYRING", Some("1".to_string())),
        ]);
        let svc = biorouter::knowledge::service::KnowledgeService::new_default().unwrap();
        make_legacy(&svc, "legacy-notes", "Legacy notes");
        let legacy_marker = svc.root().join("legacy-notes").join("legacy.txt");
        std::fs::write(&legacy_marker, "must survive a listing").unwrap();

        handle_knowledge_subcommand(KnowledgeCommand::List {
            format: "json".to_string(),
        })
        .await
        .unwrap();

        assert!(
            svc.get_base("legacy-notes").is_ok(),
            "`knowledge list` deleted a legacy base it was only asked to report"
        );
        assert_eq!(
            std::fs::read_to_string(&legacy_marker).unwrap(),
            "must survive a listing"
        );
    }

    /// The parser is internally consistent (clap's own audit: duplicate
    /// aliases, conflicting shorts, bad `value_parser`s).
    ///
    /// Not decoration — it is what makes the alias below a *parse* assertion
    /// rather than a claim about an attribute nobody built.
    #[test]
    fn the_parser_is_well_formed() {
        Cli::command().debug_assert();
    }

    /// Every path in `get_or_create_session_id` that CREATES a session row
    /// checks the provider precondition first.
    ///
    /// A fresh install has no provider, and `build_session` is where that is
    /// discovered. It runs after this function, so each attempt used to leave
    /// an orphan "CLI Session" row behind before failing. Four call sites
    /// create a row, and a fifth is exactly the kind of thing that gets added
    /// later without noticing this ordering, so the guard is structural rather
    /// than one test per path.
    ///
    /// Scanned over the function body only. The paths that return an EXISTING
    /// id are deliberately not required to check: a stored row can carry the
    /// only provider a resumed chat has, so refusing there on a missing global
    /// default would break resuming a session that works today.
    #[test]
    fn no_session_row_is_created_before_the_provider_precondition_is_checked() {
        let src = include_str!("cli.rs");
        let (_, after) = src
            .split_once("async fn get_or_create_session_id(")
            .expect("get_or_create_session_id is gone");
        let (body, _) = after
            .split_once("\nasync fn lookup_session_id(")
            .expect("could not find the end of get_or_create_session_id");

        let creates = body.match_indices(".create_session(").count();
        assert_eq!(
            creates, 4,
            "the number of row-creating paths changed; re-check that each new one \
             refuses an unconfigured run BEFORE it writes the row, then update this count"
        );

        // Every create must have the guard somewhere above it, and no two
        // creates may share one guard.
        let guards: Vec<usize> = body
            .match_indices("refuse_unconfigured_before_creating_a_row(")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            guards.len(),
            creates,
            "each row-creating path needs its own precondition check, so a fresh \
             install cannot leave an orphan session row behind for every attempt"
        );
        for (create_at, _) in body.match_indices(".create_session(") {
            assert!(
                guards.iter().any(|g| *g < create_at),
                "a create_session call at byte {create_at} has no precondition check above it"
            );
        }

        // The guard runs; the question this test could not answer is what it is
        // HANDED. Every call site passed the flags alone, so the configured
        // default never reached the precondition and a working install was
        // refused. The resolution now lives inside the guard, and this pins it
        // there: moving it back out to the call sites is exactly the change
        // that regresses.
        let (_, resolution) = src
            .split_once("fn resolution_for_a_new_row(")
            .expect("the guard no longer resolves anything");
        let (resolution, _) = resolution
            .split_once("\nfn refusal_for_a_new_row(")
            .expect("could not find the end of resolution_for_a_new_row");
        for accessor in ["get_biorouter_provider", "get_biorouter_model"] {
            assert!(
                resolution.contains(accessor),
                "a new session row must fall back to the configured default; \
                 `{accessor}` is not read where the row is guarded"
            );
        }
    }

    /// The reported bug: a configured install answered every session-creating
    /// command with "No provider is configured".
    ///
    /// Driven through the real `Config`, because the wiring between the guard
    /// and the configuration is the thing that was missing — a test of the
    /// precedence alone would have passed on the broken build too.
    /// `Config::get_param` reads the environment before the file, so this is
    /// deterministic wherever it runs.
    #[test]
    fn a_configured_default_satisfies_the_precondition_for_a_new_row() {
        let _env = env_lock::lock_env([
            ("BIOROUTER_PROVIDER", Some("claude_code".to_string())),
            ("BIOROUTER_MODEL", Some("claude-opus-5".to_string())),
        ]);

        assert_eq!(
            resolution_for_a_new_row(None, None),
            (
                Some("claude_code".to_string()),
                Some("claude-opus-5".to_string())
            ),
            "a bare `biorouter` names no provider, so the configured one is the answer"
        );
        assert_eq!(
            refusal_for_a_new_row(None, None),
            None,
            "a configured install must get past the guard with no flags at all"
        );
    }

    /// What the user typed now still wins over what they configured earlier.
    #[test]
    fn an_explicit_flag_still_outranks_the_configured_default() {
        let _env = env_lock::lock_env([
            ("BIOROUTER_PROVIDER", Some("claude_code".to_string())),
            ("BIOROUTER_MODEL", Some("claude-opus-5".to_string())),
        ]);
        assert_eq!(
            resolution_for_a_new_row(Some("anthropic"), Some("opus")),
            (Some("anthropic".to_string()), Some("opus".to_string()))
        );
    }

    /// The model has the same four slots as the provider, and shipped with the
    /// same gap: `biorouter run --provider claude_code` got past the provider
    /// half and was refused with "No model is configured".
    #[test]
    fn the_model_slot_falls_back_too() {
        assert_eq!(
            resolution_with_global_default(
                Some("claude_code"),
                None,
                Some("versa_azure".to_string()),
                Some("claude-opus-5".to_string()),
            ),
            (
                Some("claude_code".to_string()),
                Some("claude-opus-5".to_string())
            ),
            "naming a provider must not cost the configured model"
        );
    }

    /// The behaviour `5dc1eee1` added, which this fix must not undo: an install
    /// with nothing configured anywhere is still refused, and the refusal names
    /// the provider first.
    #[test]
    fn an_install_with_nothing_configured_is_still_refused() {
        assert_eq!(
            resolution_with_global_default(None, None, None, None),
            (None, None)
        );
        let text = crate::session::unconfigured_precondition(None, None)
            .expect("nothing configured anywhere must still be refused");
        assert!(text.contains("No provider is configured"), "{text}");

        // A workflow pin alone is enough — that is the whole reason `run`
        // resolves one before asking.
        assert_eq!(
            resolution_with_global_default(Some("claude_code"), Some("claude-opus-5"), None, None),
            (
                Some("claude_code".to_string()),
                Some("claude-opus-5".to_string())
            )
        );
    }

    /// The refusal tells the reader to do something they can actually do.
    ///
    /// `biorouter` with no subcommand has no `--provider` flag (`struct Cli`
    /// carries only the subcommand), and it is the command the operator's
    /// report was typed against — so the old "pass --provider <name> for this
    /// run" named a flag that does not exist on the path printing it.
    #[test]
    fn the_refusal_names_a_command_that_takes_the_flag() {
        let text = crate::session::unconfigured_precondition(None, None).expect("refused");
        for command in ["biorouter session", "biorouter run"] {
            assert!(
                text.contains(command),
                "the refusal must name a command that accepts --provider: {text}"
            );
        }
        assert!(
            Cli::try_parse_from(["biorouter", "--provider", "claude_code"]).is_err(),
            "if a global --provider is ever added, this refusal should name it instead"
        );
    }

    /// BR-71 / issue #56: `biorouter sessions <verb>` really is a command.
    ///
    /// ⚠ Driven through the real parser, on the real argv, for every verb the
    /// prose and the error messages tell people to type. An assertion that the
    /// attribute is present would pass against an alias registered on the wrong
    /// subcommand; this fails unless the argument vector a user actually types
    /// produces the subcommand they meant.
    ///
    /// `session_watch.rs`'s missing-secret error printed `biorouter sessions
    /// watch <id>` and the command reference printed it too, while `sessions`
    /// was not registered anywhere — so following the instruction gave
    /// `unrecognized subcommand`.
    #[test]
    fn sessions_is_a_real_alias_for_every_verb_the_product_prints() {
        for argv in [
            vec!["biorouter", "sessions", "watch", "20260801_7"],
            vec!["biorouter", "sessions", "send", "20260801_7", "hello"],
            vec!["biorouter", "sessions", "attach", "20260801_7"],
            vec!["biorouter", "sessions", "cancel", "20260801_7"],
            vec!["biorouter", "sessions", "list"],
        ] {
            let parsed = Cli::try_parse_from(&argv)
                .unwrap_or_else(|e| panic!("`{}` does not parse: {e}", argv.join(" ")));
            assert!(
                matches!(parsed.command, Some(Command::Session { .. })),
                "`{}` parsed as something other than the session command",
                argv.join(" ")
            );
        }

        // …and the canonical spelling and the short alias still work, so the
        // addition is additive rather than a rename.
        for name in ["session", "s"] {
            let parsed = Cli::try_parse_from(["biorouter", name, "list"])
                .unwrap_or_else(|e| panic!("`biorouter {name} list` does not parse: {e}"));
            assert!(matches!(parsed.command, Some(Command::Session { .. })));
        }
    }
}
