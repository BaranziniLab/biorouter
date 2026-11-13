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
use crate::commands::session::{
    handle_session_list, handle_session_remove, RemoveSelector, SessionScope,
};
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

/// `biorouter --version` prints `biorouter 1.91.2`, as `biorouterd` and `biorouter-crew` print
/// theirs (SF-F8). It printed ` 1.91.2`, with a leading space and no name, while an empty
/// `display_name` stood in for the name. A desktop reader pulls the number out with a pattern,
/// so the name costs it nothing.
#[derive(Parser)]
#[command(name = "biorouter", author, version, about, long_about = None)]
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

#[derive(Args, Debug, Clone, Default)]
pub struct SharedDaemonOptions {
    #[arg(
        long,
        help = "Use the profile's shared daemon; does not grant Crew access"
    )]
    pub shared_daemon: bool,
    #[arg(
        long,
        requires = "shared_daemon",
        help = "Accepted for compatibility; Biorouter no longer needs an approval secret. The first line of standard input is read and ignored"
    )]
    pub approval_key_stdin: bool,
    #[arg(
        long,
        requires = "shared_daemon",
        help = "Require an already running shared daemon"
    )]
    pub no_start: bool,
    #[arg(
        long,
        requires = "shared_daemon",
        help = "Create a daemon conversation without sending a model prompt"
    )]
    pub create_only: bool,
}

/// Session behavior options shared between Session and Run commands
#[derive(Args, Debug, Clone, Default)]
pub struct SessionOptions {
    #[command(flatten)]
    pub shared: SharedDaemonOptions,
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
        help = "Maximum number of turns allowed without user input (default: 0, unlimited)",
        long_help = "Set a limit on how many turns (iterations) the agent can take without asking for user input to continue. Use 0 for unlimited turns (the default). Existing loop detection and Stop still apply."
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
        long_help = "Override the BIOROUTER_PROVIDER environment variable for this run. Available providers include openai, anthropic, google, ollama, llamacpp, databricks, and others. Without --model, a provider other than the configured one runs its own default model."
    )]
    pub provider: Option<String>,

    /// Model to use for this run (overrides environment variable)
    #[arg(
        long = "model",
        value_name = "MODEL",
        help = "Specify the model to use (e.g., 'claude-opus-5-5', 'gpt-6-sol')",
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
async fn resolution_for_a_new_row(
    provider: Option<&str>,
    model: Option<&str>,
) -> (Option<String>, Option<String>) {
    let config = Config::global();
    let default_provider = config.get_biorouter_provider().ok();
    let default_model = config.get_biorouter_model().ok();
    // PROV-F10: the chosen provider's own default, asked for only when no model is named for
    // it, as `build_session` resolves it.
    let chosen = provider
        .map(str::to_owned)
        .or_else(|| default_provider.clone());
    let own_default = match chosen.as_deref() {
        Some(chosen)
            if crate::session::owned_model(
                Some(chosen),
                model.map(str::to_owned),
                &[(default_provider.as_deref(), default_model.clone())],
            )
            .is_none() =>
        {
            crate::session::model_for_run(Some(chosen), None, &[]).await
        }
        _ => None,
    };
    resolution_with_global_default(
        provider,
        model,
        default_provider,
        default_model,
        own_default,
    )
}

/// The precedence itself, with the configured default handed in.
///
/// Separated from [`resolution_for_a_new_row`] only so the tests can drive
/// every combination of "typed now / configured / neither" without a config
/// file and without racing the process-global `Config`.
///
/// `own_default` is the chosen provider's own default model. The configured model belongs to
/// the configured provider, so it is used only for that provider (PROV-F10); another provider
/// runs its own default, and one the registry does not know falls back to the configured model,
/// so the guard never claims "no model" for a run `build_session` would give one.
fn resolution_with_global_default(
    provider: Option<&str>,
    model: Option<&str>,
    default_provider: Option<String>,
    default_model: Option<String>,
    own_default: Option<String>,
) -> (Option<String>, Option<String>) {
    let provider = provider.map(str::to_string).or(default_provider.clone());
    let model = crate::session::owned_model(
        provider.as_deref(),
        model.map(str::to_string),
        &[(default_provider.as_deref(), default_model.clone())],
    )
    .or(own_default)
    .or(default_model);
    (provider, model)
}

/// The refusal text for a run that cannot name a provider or a model, or `None`
/// when the row is safe to create.
///
/// The exit is in the caller so that this half — which is all of the logic —
/// can be unit-tested rather than only observed by killing a process.
async fn refusal_for_a_new_row(provider: Option<&str>, model: Option<&str>) -> Option<String> {
    let (provider, model) = resolution_for_a_new_row(provider, model).await;
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
pub(crate) async fn refuse_unconfigured_before_creating_a_row(
    provider: Option<&str>,
    model: Option<&str>,
) -> Result<()> {
    if let Some(text) = refusal_for_a_new_row(provider, model).await {
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
                refuse_unconfigured_before_creating_a_row(provider, model).await?;
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
            refuse_unconfigured_before_creating_a_row(provider, model).await?;
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

        refuse_unconfigured_before_creating_a_row(provider, model).await?;
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
        refuse_unconfigured_before_creating_a_row(provider, model).await?;
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

/// ⚠ **An empty token is not a token, and `--auth-token ""` used to be accepted
/// as one.** Passing it made `validate_network_auth` see `Some(_)` and let
/// `--host 0.0.0.0` through, while `commands::web`'s middleware would then admit
/// anyone who sent `Authorization: Bearer ` with nothing after it — so the one
/// check whose entire job is to insist on protection was satisfied by its
/// absence. Refused here, at parse time, so the mistake cannot reach a bind; a
/// whitespace-only value is refused for the same reason.
pub(crate) fn parse_auth_token(s: &str) -> Result<String, String> {
    if s.trim().is_empty() {
        return Err(
            "an empty --auth-token is not a token; omit the flag to run without one (loopback \
             binds only), or pass a real secret"
                .to_string(),
        );
    }
    Ok(s.to_string())
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
                    can be reached to ask). Also lists sessions with no messages"
        )]
        subagents: bool,

        #[arg(
            long = "include-empty",
            help = "Include sessions that have not recorded a message yet — what a `biorouter` \
                    run that exited before its first prompt, `doctor --fix` and `term init` \
                    leave behind (--subagents already includes them)"
        )]
        include_empty: bool,
    },
    #[command(
        about = "Remove sessions by id, name or regex, or pick them interactively",
        long_about = "Remove sessions. --session-id removes that session whatever it is, \
                      including a subagent run or a session with no messages. --name removes the \
                      one session carrying that name and refuses when several share it. --regex \
                      removes every session whose id matches, among those `session list` shows \
                      unless --include-empty or --subagents widens the search. With none of the \
                      three, a picker opens. Asks for confirmation first unless --yes; without a \
                      terminal, --yes is required."
    )]
    Remove {
        #[command(flatten)]
        identifier: Option<Identifier>,
        #[arg(
            short = 'r',
            long,
            help = "Regex for removing matched sessions (optional)"
        )]
        regex: Option<String>,
        #[arg(
            short = 'y',
            long,
            help = "Remove without asking for confirmation (required when not run from a terminal)"
        )]
        yes: bool,
        #[arg(
            long,
            help = "With --regex or the picker: also match subagent runs (and sessions with no \
                    messages)"
        )]
        subagents: bool,
        #[arg(
            long = "include-empty",
            help = "With --regex or the picker: also match sessions that have not recorded a \
                    message yet"
        )]
        include_empty: bool,
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
            help = "Return as soon as the daemon accepts the turn, printing its turn id, instead \
                    of streaming it",
            long_help = "Return as soon as the daemon accepts the turn, printing the session and \
                         turn id, instead of streaming it. The turn runs on in the daemon: \
                         `session watch <id>` follows it and `session cancel <id>` stops it. The \
                         daemon stops a turn once nothing has been attached to its reply stream \
                         for five minutes (`session watch` does not count), so --no-wait suits \
                         turns shorter than that."
        )]
        no_wait: bool,
        #[arg(
            long,
            help = "For a daemon started with a user-action key: read the raw key from the first line of stdin, instead of being asked for it on the terminal once the daemon wants it"
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
            help = "For a daemon started with a user-action key: read the raw key from the first line of stdin, instead of being asked for it on the terminal once the daemon wants it"
        )]
        user_action_key_stdin: bool,
    },
    #[command(
        about = "List the subagents a session started, with their status, model and setup",
        long_about = "Lists the subagents a session started, oldest first: each one's status, \
                      model, extensions, skills and knowledge bases, tool calls and run time, and \
                      the first line of its result. When the session's agent declared a phase \
                      plan, the plan comes first, with each phase's progress. With a running \
                      daemon the status is live; without one the list is read from the local \
                      session store, and a subagent that was running reads as interrupted or \
                      unknown. Follow a running subagent with `biorouter session attach <id>`."
    )]
    Subagents {
        /// Session id of the parent chat.
        session_id: String,
        #[arg(
            short,
            long,
            value_enum,
            default_value_t = crate::commands::session_subagents::SubagentsFormat::Text,
            help = "Output format (text, json)"
        )]
        format: crate::commands::session_subagents::SubagentsFormat,
        #[arg(long, help = "Redraw every 2 seconds until Ctrl-C")]
        watch: bool,
        #[arg(
            long,
            help = "For a daemon started with a user-action key: read the raw key from the first line of stdin, so a private chat's subagents can be listed"
        )]
        user_action_key_stdin: bool,
    },
    #[command(
        about = "List a session's decision-model calls: aggregates first, then a page of calls",
        long_about = "Lists the decision-model calls one session made. The aggregates come \
                      first: one line per source (monitor, tool, gate, chat, adhoc) and \
                      criterion, then one line per source for each recent turn. A page of \
                      individual calls follows, newest first, with the filters below and a \
                      cursor for the next page. With a running daemon the calls come from it; \
                      without one they are read from the local session store."
    )]
    Decisions {
        /// Session id of the chat.
        session_id: String,
        #[command(flatten)]
        filters: crate::commands::session_decisions::DecisionFilters,
        #[arg(long, help = "Print JSON (the summary and the page) instead of text")]
        json: bool,
        #[arg(
            long,
            help = "For a daemon started with a user-action key: read the raw key from the first line of stdin, so a private chat's decisions can be listed"
        )]
        user_action_key_stdin: bool,
    },
    #[command(
        about = "List, add or remove the decision-model monitors that watch a session",
        long_about = "A monitor asks a decision model one question about the session's recent \
                      messages after each agent iteration, or once as a turn ends. When the \
                      probability of the wanted answer reaches its threshold, it stops the turn, \
                      shows a notice, or tells the model. With a running daemon the monitors are \
                      read and written through it; without one, in the local session store."
    )]
    Monitors {
        /// Session id of the chat.
        session_id: String,
        #[command(subcommand)]
        command: crate::commands::session_monitors::MonitorsCommand,
        #[arg(
            long,
            global = true,
            help = "For a daemon started with a user-action key: read the raw key from the first line of stdin, instead of being asked for it on the terminal"
        )]
        user_action_key_stdin: bool,
    },
    #[command(
        about = "List a session's phase gates, or set, remove, pass, skip or retry one",
        long_about = "A phase gate asks a decision model yes/no questions about a phase's \
                      hand-off before the phase may complete, and holds every phase listed \
                      after it until it passes. Whoever set a gate (the agent, you, or the \
                      workflow the chat was started from), the agent cannot change, remove or \
                      skip it: only you can, here or in the app. With no subcommand the gates \
                      are listed. With a running daemon they are read and written through it; \
                      without one, in the local session store."
    )]
    Gates {
        /// Session id of the chat whose plan holds the phases.
        session_id: String,
        #[command(subcommand)]
        command: Option<crate::commands::session_gates::GatesCommand>,
        #[arg(
            long,
            global = true,
            value_enum,
            default_value_t = crate::commands::session_gates::GatesFormat::Text,
            help = "Output format (text, json)"
        )]
        format: crate::commands::session_gates::GatesFormat,
        #[arg(
            long,
            global = true,
            help = "For a daemon started with a user-action key: read the raw key from the first line of stdin, instead of being asked for it on the terminal"
        )]
        user_action_key_stdin: bool,
    },
    #[command(about = "Stop the turn a session is running (idempotent)")]
    Cancel {
        /// Session id whose running turn should be stopped.
        session_id: String,
        #[arg(
            long,
            help = "For a daemon started with a user-action key: read the raw key from the first line of stdin, instead of being asked for it on the terminal once the daemon wants it"
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
        long_about = "Diverge a stored chat into a brand-new session. The original is left untouched. Prints the new session id to stdout (resume it with `biorouter session --resume --session-id <ID>`)."
    )]
    Diverge {
        #[command(flatten)]
        identifier: Option<Identifier>,

        /// Optional name for the new branched session
        #[arg(
            long = "branch-name",
            value_name = "NAME",
            help = "Name for the new (diverged) session"
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

    /// Decision models: fast models that answer a typed question with probabilities
    #[command(about = "List the decision models this machine can use")]
    Decision {
        #[command(subcommand)]
        command: DecisionModelCommand,
    },
}

#[derive(Subcommand)]
enum DecisionModelCommand {
    /// List the decision models available now, local first
    #[command(
        about = "List the decision models available now, with tier, context, kinds and adapter",
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
    /// Install a decision model into the Ollama on this computer
    #[command(
        about = "Install a decision model into the Ollama on this computer (default tev1:0.8b)",
        long_about = "Downloads a decision model Biorouter offers (tev1:0.8b, about 1.5 GB, or \
                      clef-flash:latest, about 12 GB) into the Ollama that OLLAMA_HOST names, \
                      which must run on this computer, and checks that Ollama answers decisions \
                      with it. A decision model on this computer is the only kind that may read \
                      a private chat. No daemon is needed: the next model listing sees it."
    )]
    Pull {
        /// The model to install: tev1:0.8b (the default) or clef-flash:latest.
        #[arg(value_name = "MODEL")]
        model: Option<String>,
    },
}

#[derive(Subcommand)]
enum DictationCommand {
    /// Whether this computer can dictate, and why: engine, processor, backend, models,
    /// policy, account and data folder
    #[command(about = "Show whether this computer can dictate, and why")]
    Status {
        #[arg(
            long = "format",
            value_name = "FORMAT",
            default_value = "text",
            value_parser = clap::builder::PossibleValuesParser::new(["text", "json"])
        )]
        format: String,

        /// Also start a worker and show its hello, or its kept stderr lines when it fails
        #[arg(short, long)]
        verbose: bool,
    },

    /// Every speech model Biorouter knows: size, languages, how well it suits this
    /// computer, and where it is installed
    #[command(
        about = "List the speech models and where they are installed",
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

    /// Download a speech model (the one recommended for this computer when MODEL is
    /// left out) and its voice detector into this account's data folder, after checking
    /// each file against the SHA-256 pinned in Biorouter
    #[command(about = "Install a speech model", visible_alias = "pull")]
    Install {
        /// A model id from `biorouter dictation list`
        #[arg(value_name = "MODEL")]
        model: Option<String>,

        /// Install files you already downloaded (see `biorouter dictation urls`); each
        /// is identified by its SHA-256, whatever its name
        #[arg(long = "from", value_name = "FILE|DIR")]
        from: Option<std::path::PathBuf>,

        /// Install into the computer's shared folder, which every account reads (needs
        /// root or Administrator)
        #[arg(long)]
        system: bool,

        /// Install without asking (needed without a terminal)
        #[arg(short, long)]
        yes: bool,

        /// Suggest dictionary words from chats after this verified install
        #[arg(long, conflicts_with = "no_suggest_from_chats")]
        suggest_from_chats: bool,

        /// Leave chat suggestions off after this verified install
        #[arg(long, conflicts_with = "suggest_from_chats")]
        no_suggest_from_chats: bool,
    },

    /// Delete an installed speech model; removing the last one removes the voice
    /// detector too
    #[command(about = "Remove an installed speech model", visible_alias = "rm")]
    Remove {
        #[arg(value_name = "MODEL")]
        model: String,

        /// Remove the copy in the computer's shared folder (needs root or Administrator)
        #[arg(long)]
        system: bool,

        /// Remove without asking (needed without a terminal)
        #[arg(short, long)]
        yes: bool,
    },

    /// Choose the speech model dictation uses
    #[command(about = "Choose the speech model dictation uses")]
    Use {
        #[arg(value_name = "MODEL")]
        model: String,
    },

    /// Print the address, size and SHA-256 of a model and of every file it needs, to
    /// download them on another computer and install them with `install --from`
    #[command(about = "Print the download addresses of a model's files")]
    Urls {
        #[arg(value_name = "MODEL")]
        model: Option<String>,
    },

    /// Hash every installed model file again; a copy in this account's folder that
    /// fails is deleted
    #[command(about = "Check every installed model file against its pin")]
    Verify {},

    /// Time the model in use on a 5.5 s recording and record how fast this computer runs
    /// it
    #[command(about = "Measure how fast this computer transcribes")]
    Bench {
        #[arg(
            long = "format",
            value_name = "FORMAT",
            default_value = "text",
            value_parser = clap::builder::PossibleValuesParser::new(["text", "json"])
        )]
        format: String,
    },

    /// Load a model and decode once, so the first dictation does not wait for the GPU's
    /// shaders to compile. `install` starts it in the background.
    #[command(name = "warm-up", hide = true)]
    WarmUp {
        #[arg(value_name = "MODEL")]
        model: String,
    },

    /// The microphones `biorouter dictate` can use, listed without opening any; choose one
    /// with BIOROUTER_CLI_DICTATION_DEVICE
    #[command(about = "List the microphones dictation can use")]
    Devices {
        #[arg(
            long = "format",
            value_name = "FORMAT",
            default_value = "text",
            value_parser = clap::builder::PossibleValuesParser::new(["text", "json"])
        )]
        format: String,
    },

    /// Record from the microphone for a few seconds and report what arrived: the device,
    /// the level, and whether the operating system let any sound through. Nothing is
    /// transcribed or kept. Run it from your own terminal: it is how macOS first asks your
    /// terminal app for the microphone.
    #[command(about = "Check the microphone without transcribing")]
    Test {
        /// How long to record, at most 600 (default 5)
        #[arg(long, value_name = "N", default_value_t = 5, value_parser = clap::value_parser!(u32).range(1..=600))]
        seconds: u32,
    },
}

/// `biorouter dictionary` (DD-75). Writes need a person at an interactive terminal,
/// except `import`; none runs in an agent's shell.
#[derive(Subcommand)]
enum DictionaryCommand {
    /// The terms in this account's dictionary. Without a terminal only the public terms
    /// are listed
    #[command(about = "List the dictionary's terms", visible_alias = "ls")]
    List {
        #[arg(
            long = "format",
            value_name = "FORMAT",
            default_value = "text",
            value_parser = clap::builder::PossibleValuesParser::new(["text", "json", "csv"])
        )]
        format: String,

        #[arg(
            long = "status",
            value_name = "STATUS",
            default_value = "all",
            value_parser = clap::builder::PossibleValuesParser::new(["active", "suggested", "all"])
        )]
        status: String,
    },

    /// Add words dictation should spell your way, such as UCSF or EPAS1
    #[command(about = "Add terms to the dictionary")]
    Add {
        #[arg(value_name = "TERM", required = true)]
        terms: Vec<String>,

        /// How you say the term, comma separated; may repeat
        #[arg(long = "spoken", value_name = "FORMS")]
        spoken: Vec<String>,

        /// Use the terms only when dictating into chats on private models
        #[arg(long)]
        private: bool,
    },

    /// Change a term's spelling or how you say it
    #[command(
        about = "Change a term",
        group(clap::ArgGroup::new("change").required(true).multiple(true).args(["new_term", "spoken"]))
    )]
    Edit {
        #[arg(value_name = "TERM", required_unless_present = "id")]
        term: Option<String>,

        /// The entry's id, for a term that is both public and private
        #[arg(long, value_name = "ID")]
        id: Option<String>,

        /// The new spelling
        #[arg(long = "term", value_name = "NEW")]
        new_term: Option<String>,

        /// How you say the term, comma separated; replaces the spoken forms
        #[arg(long = "spoken", value_name = "FORMS")]
        spoken: Option<Vec<String>>,
    },

    /// Delete terms
    #[command(
        about = "Remove terms from the dictionary",
        visible_alias = "rm",
        group(clap::ArgGroup::new("which").required(true).multiple(true).args(["terms", "ids"]))
    )]
    Remove {
        #[arg(value_name = "TERM")]
        terms: Vec<String>,

        /// An entry's id; may repeat
        #[arg(long = "id", value_name = "ID")]
        ids: Vec<String>,
    },

    /// Accept or dismiss the suggestions and the agent's terms waiting for you
    #[command(about = "Review suggested terms")]
    Review {},

    /// Accept suggestions by id
    #[command(about = "Accept suggested terms")]
    Accept {
        #[arg(value_name = "ID", required = true)]
        ids: Vec<String>,
    },

    /// Dismiss suggestions by id; they are not suggested again
    #[command(about = "Dismiss suggested terms")]
    Dismiss {
        #[arg(value_name = "ID", required = true)]
        ids: Vec<String>,
    },

    /// Merge terms from a file or standard input: one term per line
    /// (`EPAS1 = epas one, e pass one`), CSV or TSV with the header
    /// `written,spoken_forms,note`, or an export. Never deletes
    #[command(about = "Import terms")]
    Import {
        /// The file, or - for standard input
        #[arg(value_name = "FILE|-")]
        input: String,

        /// The file's format (default: from its extension; standard input needs it
        /// unless it is an export)
        #[arg(
            long = "format",
            value_name = "FORMAT",
            value_parser = clap::builder::PossibleValuesParser::new(["txt", "csv", "tsv", "json"])
        )]
        format: Option<String>,

        /// Print what would change, and change nothing
        #[arg(long)]
        dry_run: bool,
    },

    /// Write the terms as JSON, for `import` on another computer. Without ids or usage
    #[command(about = "Export terms")]
    Export {
        /// The file, or - for standard output (the default)
        #[arg(value_name = "FILE|-")]
        output: Option<String>,

        /// Include private terms, after a confirmation (needs a terminal and a private
        /// model configured)
        #[arg(long)]
        include_private: bool,
    },

    /// Let a private term shape dictation into chats on public models
    #[command(about = "Make a private term public")]
    Publicize {
        #[arg(value_name = "TERM", required_unless_present = "id")]
        term: Option<String>,

        #[arg(long, value_name = "ID")]
        id: Option<String>,
    },

    /// Use a term only when dictating into chats on private models
    #[command(about = "Make a public term private")]
    Privatize {
        #[arg(value_name = "TERM", required_unless_present = "id")]
        term: Option<String>,

        #[arg(long, value_name = "ID")]
        id: Option<String>,
    },

    /// Show the dictionary's settings, or change them
    #[command(about = "Show or change the dictionary's settings")]
    Settings {
        /// What the agent's terms do: wait for you (suggest), go in when you ask
        /// (on_request), or go in at once (add)
        #[arg(
            long = "agent-writes",
            value_name = "MODE",
            value_parser = clap::builder::PossibleValuesParser::new(["suggest", "on_request", "add"])
        )]
        agent_writes: Option<String>,

        /// Suggest words from the messages you type in chats on public models, on this
        /// computer; you keep or dismiss each one
        #[arg(
            long = "learn-from-chats",
            value_name = "ON|OFF",
            value_parser = clap::builder::PossibleValuesParser::new(["on", "off"])
        )]
        learn_from_chats: Option<String>,

        /// Let the agent suggest words it sees in your chats
        #[arg(long = "suggest-from-chats", value_name = "ON|OFF",
              value_parser = clap::builder::PossibleValuesParser::new(["on", "off"]))]
        suggest_from_chats: Option<String>,
    },

    /// Delete every term, after you type the count
    #[command(about = "Delete every term")]
    Clear {
        /// Delete only the suggestions
        #[arg(long)]
        suggested_only: bool,
    },

    /// The words waiting for you to keep or dismiss: words found in your own messages and
    /// the agent's suggested terms. Without a terminal only the public ones are listed
    #[command(about = "List, keep, dismiss or find suggested words")]
    Suggestions {
        #[command(subcommand)]
        command: Option<SuggestionsCommand>,

        #[arg(
            long = "format",
            value_name = "FORMAT",
            value_parser = clap::builder::PossibleValuesParser::new(["text", "json"])
        )]
        format: Option<String>,
    },
}

/// `biorouter dictionary suggestions`. Keeping, dismissing and finding words need a person
/// at an interactive terminal; listing does not.
#[derive(Subcommand)]
enum SuggestionsCommand {
    /// The words waiting for you (what `suggestions` alone prints)
    #[command(
        about = "List the suggested words waiting for you",
        visible_alias = "ls"
    )]
    List {
        #[arg(
            long = "format",
            value_name = "FORMAT",
            value_parser = clap::builder::PossibleValuesParser::new(["text", "json"])
        )]
        format: Option<String>,
    },

    /// Keep suggested words, named by the word or by id
    #[command(about = "Keep suggested words")]
    Keep {
        #[arg(value_name = "TERM|ID", required = true)]
        terms: Vec<String>,
    },

    /// Dismiss suggested words, named by the word or by id; a dismissed word is never
    /// suggested again
    #[command(about = "Dismiss suggested words")]
    Dismiss {
        #[arg(value_name = "TERM|ID", required = true)]
        terms: Vec<String>,
    },

    /// Read your own chats on public models from the last 30 days (at most 100) once, on
    /// this computer, and suggest the words that show up in several of them. Ctrl+C stops
    /// it after the chat it is reading and keeps what it found
    #[command(about = "Find words in your recent chats")]
    Scan {},
}

impl SuggestionsCommand {
    fn into_action(
        command: Option<Self>,
        format: Option<String>,
    ) -> crate::commands::dictionary::Action {
        use crate::commands::dictionary::{Action, ListFormat};
        let listed = |format: Option<String>| Action::Suggestions {
            format: match format.as_deref() {
                Some("json") => ListFormat::Json,
                _ => ListFormat::Text,
            },
        };
        match command {
            None => listed(format),
            Some(Self::List { format: inner }) => listed(inner.or(format)),
            Some(Self::Keep { terms }) => Action::KeepSuggestions { terms },
            Some(Self::Dismiss { terms }) => Action::DismissSuggestions { terms },
            Some(Self::Scan {}) => Action::ScanSuggestions,
        }
    }
}

impl DictionaryCommand {
    fn into_action(self) -> crate::commands::dictionary::Action {
        use crate::commands::dictionary::{Action, ListFormat, StatusFilter};
        match self {
            Self::List { format, status } => Action::List {
                format: match format.as_str() {
                    "json" => ListFormat::Json,
                    "csv" => ListFormat::Csv,
                    _ => ListFormat::Text,
                },
                status: match status.as_str() {
                    "active" => StatusFilter::Active,
                    "suggested" => StatusFilter::Suggested,
                    _ => StatusFilter::All,
                },
            },
            Self::Add {
                terms,
                spoken,
                private,
            } => Action::Add {
                terms,
                spoken,
                private,
            },
            Self::Edit {
                term,
                id,
                new_term,
                spoken,
            } => Action::Edit {
                term,
                id,
                new_term,
                spoken,
            },
            Self::Remove { terms, ids } => Action::Remove { terms, ids },
            Self::Review {} => Action::Review,
            Self::Accept { ids } => Action::Accept { ids },
            Self::Dismiss { ids } => Action::Dismiss { ids },
            Self::Import {
                input,
                format,
                dry_run,
            } => Action::Import {
                input,
                format,
                dry_run,
            },
            Self::Export {
                output,
                include_private,
            } => Action::Export {
                output,
                include_private,
            },
            Self::Publicize { term, id } => Action::Publicize { term, id },
            Self::Privatize { term, id } => Action::Privatize { term, id },
            Self::Settings {
                agent_writes,
                learn_from_chats,
                suggest_from_chats,
            } => Action::Settings {
                agent_writes: agent_writes
                    .as_deref()
                    .and_then(biorouter::security::dictionary_store::store::AgentWrites::parse),
                learn_from_chats: learn_from_chats.map(|value| value == "on"),
                suggest_from_chats: suggest_from_chats.map(|value| value == "on"),
            },
            Self::Clear { suggested_only } => Action::Clear { suggested_only },
            Self::Suggestions { command, format } => {
                SuggestionsCommand::into_action(command, format)
            }
        }
    }
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
        #[arg(
            help = "Catalog model name (qwen3.8-27b or gemma4-31b) or a raw owner/repo:QUANT HF spec; a retired catalog name downloads the model a chat naming it runs"
        )]
        model: String,
    },

    /// Delete a downloaded local model to reclaim disk space
    #[command(about = "Delete a downloaded local model", visible_alias = "delete")]
    Rm {
        #[arg(
            help = "Catalog model name (e.g. qwen3.8-27b), a retired catalog name (e.g. gemma4) to free the files it left, or a raw owner/repo:QUANT HF spec"
        )]
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

#[derive(Args)]
struct ServeOptions {
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
    #[arg(
        long,
        help = "Use this access token instead of generating one. Takes precedence over \
                BIOROUTER_BROWSER_TOKEN, which is read when this is not given."
    )]
    token: Option<String>,

    /// Serve without an access token
    #[arg(
        long,
        conflicts_with = "token",
        help = "Serve without an access token. Refused for a non-loopback bind."
    )]
    no_token: bool,

    /// Keep model installation on the host, not in a served browser
    #[arg(
        long,
        help = "Disable speech model installation from this browser session"
    )]
    no_dictation_install: bool,

    /// Directory holding the built interface
    #[arg(
        long,
        help = "Directory holding the built web interface. Takes precedence over \
                BIOROUTER_SERVE_UI; either must contain an index.html"
    )]
    web_dir: Option<std::path::PathBuf>,

    /// The address a reverse proxy serves the interface at
    #[arg(
        long,
        value_name = "URL",
        help = "The https:// address a reverse proxy serves the interface at, path prefix \
                included (for example https://hub.example.org/user/me/proxy/8765/). The proxy \
                must strip the prefix. Printed with the access token, and admitted by the \
                live views' origin check. Takes precedence over BIOROUTER_PUBLIC_URL."
    )]
    public_url: Option<String>,

    /// Configure a separate approval key for Biorouter Copilot tasks in the browser
    #[arg(
        long,
        help = "Interactively set a Biorouter Copilot approval key; requires a terminal"
    )]
    computer_use_approval: bool,

    /// Configure a separate key for external-folder decisions in the browser
    #[arg(
        long,
        help = "Interactively set a folder-access approval key; requires a terminal"
    )]
    folder_access_approval: bool,

    /// Open a browser once it is ready
    #[arg(long, help = "Open a browser once the server is ready")]
    open: bool,
}

impl ServeOptions {
    async fn run(self) -> anyhow::Result<()> {
        crate::commands::serve::handle_serve(
            self.host,
            self.port,
            self.token,
            self.no_token,
            self.no_dictation_install,
            self.web_dir,
            self.public_url,
            self.open,
            self.computer_use_approval,
            self.folder_access_approval,
        )
        .await
    }
}

#[derive(Subcommand)]
enum Command {
    /// Collaborate through Crew using the shared local daemon.
    Crew(crate::commands::crew::CrewOptions),
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
    //
    // ⚠ `sessions` is an alias, and it is not cosmetic. `session_watch.rs`'s
    // own error message told users to run `biorouter sessions watch <id>`, the
    // BR-71 plan wrote `biorouter sessions …` in roughly forty places, and
    // `docs/cli/command-reference.md` printed it — while the plural was never a
    // registered command, so every one of those instructions ended in
    // `unrecognized subcommand 'sessions'`. Registering it makes the
    // instructions true rather than making forty documents wrong.
    //
    // A `//` comment, not `///`: clap turns a doc comment into the command's long
    // help, and this note printed on every `biorouter session --help` (AG-F8).
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

        /// Run the chat in this process even when the shared daemon is running
        #[arg(
            long,
            conflicts_with = "shared_daemon",
            help = "Run the chat in this terminal only, not on the shared Biorouter daemon",
            long_help = "Run the chat in this terminal only. Without it, an interactive chat runs on the shared Biorouter daemon when one is running, so it can use Crew as you and shows in the desktop app."
        )]
        local: bool,

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

    /// Ask a decision model one typed question about a text
    #[command(
        about = "Ask a decision model one typed question about a text, with probabilities",
        long_about = "Asks a decision model one question about a text and prints the answer \
                      with the probability of every outcome, drawn as bars. The question is yes \
                      or no by default; --choice makes it a choice between named options and \
                      --levels a score. The text comes from stdin, --text, or one file per \
                      decision with --batch <DIR>, which also prints a summary table. With \
                      --session the decision is recorded in that chat's decision log, and the \
                      chat's privacy tier decides which decision models may read the text. \
                      Without one, a model hosted outside your institution is named in a \
                      warning before anything is sent."
    )]
    Decide(crate::commands::decide::DecideOptions),

    /// Inspect and update model/provider configuration
    #[command(about = "Inspect and update model/provider configuration")]
    Models {
        #[command(subcommand)]
        command: ModelsCommand,
    },

    /// Local dictation: engine status and speech models
    #[command(about = "Local dictation: engine status and speech models")]
    Dictation {
        #[command(subcommand)]
        command: DictationCommand,
    },

    /// Turn a recording into text on this computer, with the speech model dictation uses.
    /// Reads WAV, AIFF, FLAC and MP3 (convert anything else with ffmpeg); nothing leaves
    /// this computer.
    /// Exit codes: 0 text, 1 failed, 2 declined to start, 3 nothing heard, 4 cancelled.
    #[command(about = "Transcribe a recording on this computer")]
    Transcribe {
        /// The recording, or - to read it from standard input
        #[arg(value_name = "FILE|-")]
        input: String,

        #[arg(
            long = "format",
            value_name = "FORMAT",
            default_value = "text",
            value_parser = clap::builder::PossibleValuesParser::new(["text", "json", "srt", "vtt"])
        )]
        format: String,

        /// Write the transcript to FILE instead of standard output
        #[arg(short, long, value_name = "FILE")]
        output: Option<std::path::PathBuf>,

        /// The language spoken: a two letter code such as en, or auto (default: the
        /// language set for dictation, en unless changed)
        #[arg(long, value_name = "LANGUAGE")]
        language: Option<String>,

        /// A speech model id from `biorouter dictation list` (default: the model in use)
        #[arg(long, value_name = "MODEL")]
        model: Option<String>,

        /// The chat the transcript is for: its private dictionary terms are used only when
        /// the chat is private, you run this at a terminal, and a private model is configured
        #[arg(long, value_name = "SESSION_ID")]
        session: Option<String>,

        /// Text the recording is about (at most 4 KB read), which ranks the dictionary
        /// terms that bias the transcript; it is never sent anywhere
        #[arg(long = "context-file", value_name = "FILE")]
        context_file: Option<std::path::PathBuf>,

        /// Transcribe without the personal dictionary
        #[arg(long = "no-dictionary")]
        no_dictionary: bool,
    },

    /// Dictate from this terminal's microphone with the local engine and your dictionary.
    /// Press Enter to stop; the text is shown for review (Enter uses it, e edits it, Esc
    /// discards it), then printed, so "$(biorouter dictate)" and pipes send nothing you have
    /// not seen. Runs only at your own terminal, never in an agent's shell.
    /// Exit codes: 0 text, 1 failed, 2 declined to start, 3 nothing heard, 4 cancelled,
    /// 5 --copy found no clipboard and printed the text.
    #[command(about = "Dictate from this terminal's microphone")]
    Dictate {
        /// Put the text on the system clipboard instead of standard output
        #[arg(long)]
        copy: bool,

        /// With --copy: ask clipboard managers not to keep the text in their history
        #[arg(long = "no-history")]
        no_history: bool,

        #[arg(
            long = "format",
            value_name = "FORMAT",
            default_value = "text",
            value_parser = clap::builder::PossibleValuesParser::new(["text", "json"])
        )]
        format: String,

        /// Stop after N seconds, at most 600 (the limit of every recording)
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=600))]
        seconds: Option<u32>,

        /// The chat the text is for: its private dictionary terms are used only when the
        /// chat is private and a private model is configured
        #[arg(long, value_name = "SESSION_ID")]
        session: Option<String>,

        /// Let your private dictionary terms shape this one recording, after a warning:
        /// only at your own terminal, with a private model configured
        #[arg(long = "private-terms")]
        private_terms: bool,

        /// The language spoken: a two letter code such as en, or auto (default: the
        /// language set for dictation, en unless changed)
        #[arg(long, value_name = "LANGUAGE")]
        language: Option<String>,

        /// A speech model id from `biorouter dictation list` (default: the model in use)
        #[arg(long, value_name = "MODEL")]
        model: Option<String>,

        /// Dictate without the personal dictionary
        #[arg(long = "no-dictionary")]
        no_dictionary: bool,

        /// Print or copy the text without showing it for review first
        #[arg(long = "no-review")]
        no_review: bool,

        /// The same as --no-review
        #[arg(short, long)]
        yes: bool,

        /// Transcribe a recording instead: the same as `biorouter transcribe FILE`
        #[arg(long, value_name = "FILE", hide = true)]
        file: Option<String>,
    },

    /// The personal dictionary dictation spells your words from, kept on this computer
    #[command(about = "Manage the words dictation spells your way")]
    Dictionary {
        #[command(subcommand)]
        command: DictionaryCommand,
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
    Serve(ServeOptions),

    /// Deprecated: use `biorouter serve`
    ///
    /// This is an older, hand-written chat page -- not the Biorouter
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
        #[arg(
            long,
            value_parser = parse_auth_token,
            help = "Authentication token to secure the web interface"
        )]
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
        Some(Command::Crew(_)) => "crew",
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
        Some(Command::Dictation { .. }) => "dictation",
        Some(Command::Transcribe { .. }) => "transcribe",
        Some(Command::Dictate { .. }) => "dictate",
        Some(Command::Dictionary { .. }) => "dictionary",
        Some(Command::Decide(_)) => "decide",
        Some(Command::Knowledge { .. }) => "knowledge",
        Some(Command::Extension { .. }) => "extension",
        Some(Command::Skill { .. }) => "skill",
        Some(Command::Apps { .. }) => "apps",
        Some(Command::Doctor { .. }) => "doctor",
        Some(Command::SetupPath { .. }) => "setup-path",
        Some(Command::Bench { .. }) => "bench",
        Some(Command::Workflow { .. }) => "workflow",
        Some(Command::Serve(_)) => "serve",
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
        McpCommand::WebDocuments => serve(biorouter_mcp::WebDocumentsServer::new()).await?,
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
            include_empty,
        } => {
            let scope = SessionScope {
                subagents,
                include_empty,
            };
            handle_session_list(format, ascending, working_dir, limit, scope).await?;
        }
        SessionCommand::Remove {
            identifier,
            regex,
            yes,
            subagents,
            include_empty,
        } => {
            let (session_id, name) = if let Some(id) = identifier {
                (id.session_id, id.name)
            } else {
                (None, None)
            };
            let scope = SessionScope {
                subagents,
                include_empty,
            };
            handle_session_remove(
                RemoveSelector::from_flags(session_id, name, regex),
                scope,
                yes,
            )
            .await?;
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
        SessionCommand::Subagents {
            session_id,
            format,
            watch,
            user_action_key_stdin,
        } => {
            crate::commands::session_subagents::handle_session_subagents(
                &session_id,
                format,
                watch,
                user_action_key_stdin,
            )
            .await?;
        }
        SessionCommand::Decisions {
            session_id,
            filters,
            json,
            user_action_key_stdin,
        } => {
            crate::commands::session_decisions::handle_session_decisions(
                &session_id,
                filters,
                json,
                user_action_key_stdin,
            )
            .await?;
        }
        SessionCommand::Monitors {
            session_id,
            command,
            user_action_key_stdin,
        } => {
            crate::commands::session_monitors::handle_session_monitors(
                &session_id,
                command,
                user_action_key_stdin,
            )
            .await?;
        }
        SessionCommand::Gates {
            session_id,
            command,
            format,
            user_action_key_stdin,
        } => {
            crate::commands::session_gates::handle_session_gates(
                &session_id,
                command,
                format,
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

fn reject_shared_fields(fields: &[(&str, bool)]) -> Result<()> {
    let unsupported: Vec<_> = fields
        .iter()
        .filter_map(|(name, present)| present.then_some(*name))
        .collect();
    anyhow::ensure!(
        unsupported.is_empty(),
        "--shared-daemon does not support: {}",
        unsupported.join(", ")
    );
    Ok(())
}

/// The options only a chat in this process takes, each with whether it was given. A shared
/// daemon chat refuses them, and an interactive `biorouter session` given one stays in this
/// process (D4).
fn in_process_fields(
    identifier: &Option<Identifier>,
    session: &SessionOptions,
    extensions: &ExtensionOptions,
) -> [(&'static str, bool); 8] {
    [
        (
            "--name",
            identifier.as_ref().is_some_and(|id| id.name.is_some()),
        ),
        (
            "--path",
            identifier.as_ref().is_some_and(|id| id.path.is_some()),
        ),
        ("--debug", session.debug),
        (
            "--max-tool-repetitions",
            session.max_tool_repetitions.is_some(),
        ),
        ("--max-turns", session.max_turns.is_some()),
        ("--with-extension", !extensions.extensions.is_empty()),
        (
            "--with-streamable-http-extension",
            !extensions.streamable_http_extensions.is_empty(),
        ),
        ("--with-builtin", !extensions.builtins.is_empty()),
    ]
}

fn shared_conversation_options(
    identifier: &Option<Identifier>,
    resume: bool,
    session: &SessionOptions,
    extensions: &ExtensionOptions,
    model: &ModelOptions,
) -> Result<crate::commands::shared_conversation::SharedConversationOptions> {
    reject_shared_fields(&in_process_fields(identifier, session, extensions))?;
    let session_id = identifier.as_ref().and_then(|id| id.session_id.clone());
    anyhow::ensure!(
        !resume || session_id.is_some(),
        "--shared-daemon --resume requires an exact --session-id; local session lookup is not used"
    );
    anyhow::ensure!(
        session_id.is_none() || resume,
        "--session-id requires --resume"
    );
    anyhow::ensure!(
        !session.shared.create_only || session_id.is_none(),
        "--create-only creates a new conversation and cannot be used with --resume/--session-id"
    );
    if session_id.is_none() {
        anyhow::ensure!(
            model
                .provider
                .as_ref()
                .is_some_and(|v| !v.trim().is_empty())
                && model.model.as_ref().is_some_and(|v| !v.trim().is_empty()),
            "A new --shared-daemon conversation requires both --provider and --model"
        );
    }
    Ok(
        crate::commands::shared_conversation::SharedConversationOptions {
            session_id,
            working_dir: std::env::current_dir()?,
            provider: model.provider.clone(),
            model: model.model.clone(),
            prompt: None,
            interactive: false,
            create_only: session.shared.create_only,
            history: false,
            approval_key_stdin: session.shared.approval_key_stdin,
            no_start: session.shared.no_start,
            quiet: false,
            output_format: "text".into(),
        },
    )
}

async fn handle_shared_run(
    input: &InputOptions,
    identifier: &Option<Identifier>,
    behavior: &RunBehavior,
    session: &SessionOptions,
    extensions: &ExtensionOptions,
    output: &OutputOptions,
    model: &ModelOptions,
) -> Result<()> {
    reject_shared_fields(&[
        ("--workflow", input.workflow.is_some()),
        ("--system", input.system.is_some()),
        ("--params", !input.params.is_empty()),
        ("--sub-workflow", !input.additional_sub_workflows.is_empty()),
        ("--explain", input.explain),
        ("--render-workflow", input.render_workflow),
        ("--no-session", behavior.no_session),
        ("--scheduled-job-id", behavior.scheduled_job_id.is_some()),
    ])?;
    let mut options =
        shared_conversation_options(identifier, behavior.resume, session, extensions, model)?;
    let has_input = input.instructions.is_some() || input.input_text.is_some();
    anyhow::ensure!(
        !options.create_only || (!has_input && !behavior.interactive),
        "--create-only cannot be combined with --text, --instructions, or --interactive"
    );
    anyhow::ensure!(!(options.approval_key_stdin && input.instructions.as_deref() == Some("-")), "--approval-key-stdin and --instructions - cannot share stdin; provide --text or an instruction file");
    anyhow::ensure!(
        has_input || options.create_only || behavior.interactive,
        "--shared-daemon run requires --text, --instructions, --interactive, or --create-only"
    );
    if has_input {
        options.prompt =
            parse_run_input(input, output.quiet)?.and_then(|(config, _)| config.contents);
    }
    options.interactive = behavior.interactive;
    options.quiet = output.quiet;
    options.output_format = output.output_format.clone();
    crate::commands::shared_conversation::run(options).await
}

/// `biorouter session` — both of its forms, dispatched in one place. The interactive form
/// runs its chat on the shared daemon when one is running and answers its saved key, and in
/// this process otherwise or with `--local` (D4, [`shared_daemon_for`]).
///
/// ⚠ **Extracted from `cli()` rather than inlined, and the reason is mechanical.**
/// Task 31 (issue #56) gave the interactive form a seventh field, `model_opts`,
/// which pushed the call past one line and `cli()` past clippy's 100-line
/// ceiling (`too_many_lines`, 103/100 — a NEW violation against
/// `scripts/clippy-lint.sh`'s baseline, so CI fails on it). The honest fix is to
/// move the arm's body out, not to `#[allow]` the lint on a function that has
/// been growing an arm per feature for years. The chat's fields travel as one
/// [`SessionChat`] since D4 added an eighth, `local`.
async fn handle_session_command(command: Option<SessionCommand>, chat: SessionChat) -> Result<()> {
    if chat.session_opts.shared.shared_daemon {
        anyhow::ensure!(command.is_none(), "--shared-daemon on session does not support local session subcommands; use an exact --resume --session-id for a daemon conversation");
        return handle_shared_session(chat).await;
    }
    match command {
        Some(cmd) => handle_session_subcommand(cmd).await,
        None => match shared_daemon_for(&chat).await {
            Some(client) => run_default_shared_session(client, chat).await,
            None => handle_interactive_session(chat).await,
        },
    }
}

/// What `biorouter session` without a subcommand was asked for: one chat, opened here.
#[derive(Default)]
struct SessionChat {
    identifier: Option<Identifier>,
    resume: bool,
    history: bool,
    /// `--local`: the chat runs in this process even when the shared daemon is running.
    local: bool,
    session_opts: SessionOptions,
    extension_opts: ExtensionOptions,
    model_opts: ModelOptions,
}

/// What [`crate::commands::session::shared_daemon_for_chat`] decides on for `chat`, read from
/// its options and this process.
fn chat_request(chat: &SessionChat) -> crate::commands::session::ChatRequest {
    use crate::commands::session::{chat_model, configured_model, ChatRequest};
    let session_id = chat
        .identifier
        .as_ref()
        .and_then(|id| id.session_id.as_deref());
    let (provider, model) = (
        chat.model_opts.provider.as_deref(),
        chat.model_opts.model.as_deref(),
    );
    let model_known = match session_id {
        // A chat resumed by its ID keeps its own model unless both are named.
        Some(_) => provider.is_some() == model.is_some(),
        None => chat_model(provider, model, configured_model).is_some(),
    };
    ChatRequest {
        local: chat.local,
        at_a_terminal: crate::commands::session::at_a_terminal(),
        // `--session-id` without `--resume` is refused by the chat in this process.
        in_process_option: (session_id.is_some() && !chat.resume)
            || in_process_fields(&chat.identifier, &chat.session_opts, &chat.extension_opts)
                .iter()
                .any(|(_, given)| *given),
        model_known,
        agent_shell: crate::commands::needs_terminal::in_agent_shell(),
    }
}

/// D4: the shared daemon an interactive `chat` runs on, or `None` to run it in this process.
async fn shared_daemon_for(chat: &SessionChat) -> Option<crate::daemon_client::CrewClient> {
    crate::commands::session::shared_daemon_for_chat(&chat_request(chat)).await
}

/// Run `chat` on the shared daemon `client` reached (D4): the chat `--session-id` names, the
/// latest one for `--resume` alone (found as an in-process chat finds it), or a new one.
async fn run_default_shared_session(
    client: crate::daemon_client::CrewClient,
    chat: SessionChat,
) -> Result<()> {
    use crate::commands::session::{chat_model, configured_model, latest_chat_id};
    eprintln!("{}", crate::commands::session::RUNS_ON_THE_SHARED_DAEMON);
    let SessionChat {
        identifier,
        resume,
        history,
        model_opts,
        ..
    } = chat;
    let mut session_id = identifier.and_then(|id| id.session_id);
    if resume && session_id.is_none() {
        session_id = latest_chat_id().await?;
        if session_id.is_none() {
            eprintln!("No previous chat to resume; starting a new chat.");
        }
    }
    let (provider, model) = match &session_id {
        Some(_) => (model_opts.provider, model_opts.model),
        None => chat_model(
            model_opts.provider.as_deref(),
            model_opts.model.as_deref(),
            configured_model,
        )
        .unzip(),
    };
    let options = crate::commands::shared_conversation::SharedConversationOptions {
        session_id,
        working_dir: std::env::current_dir()?,
        provider,
        model,
        prompt: None,
        interactive: true,
        create_only: false,
        history,
        approval_key_stdin: false,
        no_start: true,
        quiet: false,
        output_format: "text".into(),
    };
    run_shared_chat(options, Some(client)).await
}

/// `biorouter session --shared-daemon`. A new interactive chat starts on the configured
/// provider and model when neither `--provider` nor `--model` is given, as an in-process chat
/// does; `--create-only` still needs both.
async fn handle_shared_session(chat: SessionChat) -> Result<()> {
    let SessionChat {
        identifier,
        resume,
        history,
        session_opts,
        extension_opts,
        model_opts,
        ..
    } = chat;
    let new_interactive_chat = !session_opts.shared.create_only
        && !resume
        && identifier.as_ref().is_none_or(|id| id.session_id.is_none());
    let model_opts = new_chat_model(
        model_opts,
        new_interactive_chat,
        crate::commands::session::configured_model,
    );
    let mut options = shared_conversation_options(
        &identifier,
        resume,
        &session_opts,
        &extension_opts,
        &model_opts,
    )?;
    options.interactive = !options.create_only;
    options.history = history;
    run_shared_chat(options, None).await
}

/// `model_opts` for a `biorouter session --shared-daemon` chat: a new interactive chat given
/// neither `--provider` nor `--model` takes the `configured` pair, as an in-process chat does.
/// Anything else keeps what was given, and the conversation checks it.
fn new_chat_model(
    model_opts: ModelOptions,
    new_interactive_chat: bool,
    configured: impl FnOnce() -> (Option<String>, Option<String>),
) -> ModelOptions {
    if !new_interactive_chat || model_opts.provider.is_some() || model_opts.model.is_some() {
        return model_opts;
    }
    match crate::commands::session::chat_model(None, None, configured) {
        Some((provider, model)) => ModelOptions {
            provider: Some(provider),
            model: Some(model),
        },
        None => model_opts,
    }
}

/// The provider and model of a new chat this command starts on the shared daemon itself
/// ([`run_shared_chat`]): a new interactive chat at a terminal (`at_a_terminal`: standard
/// input and error) that names both. `None` leaves the chat to the conversation exactly as
/// given, which refuses one it cannot hold before it creates anything.
fn starts_here(
    options: &crate::commands::shared_conversation::SharedConversationOptions,
    at_a_terminal: bool,
) -> Option<(String, String)> {
    if options.session_id.is_some()
        || !options.interactive
        || options.approval_key_stdin
        || !at_a_terminal
    {
        return None;
    }
    Some((options.provider.clone()?, options.model.clone()?))
}

/// Run a shared daemon conversation. A new interactive chat ([`starts_here`]) is started first,
/// in this folder with the configured extensions as an in-process chat starts
/// ([`crate::commands::session::start_daemon_chat`]), and then opened by its ID.
async fn run_shared_chat(
    mut options: crate::commands::shared_conversation::SharedConversationOptions,
    client: Option<crate::daemon_client::CrewClient>,
) -> Result<()> {
    use std::io::IsTerminal;
    let at_a_terminal = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if let Some((provider, model)) = starts_here(&options, at_a_terminal) {
        let client = match client {
            Some(client) => client,
            None => crate::daemon_client::CrewClient::connect(options.no_start).await?,
        };
        let id = crate::commands::session::start_daemon_chat(
            &client,
            &options.working_dir,
            &provider,
            &model,
        )
        .await?;
        options.session_id = Some(id);
    }
    crate::commands::shared_conversation::run(options).await
}

async fn handle_interactive_session(chat: SessionChat) -> Result<()> {
    let SessionChat {
        identifier,
        resume,
        history,
        session_opts,
        extension_opts,
        model_opts,
        ..
    } = chat;
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
    if session_opts.shared.shared_daemon {
        return handle_shared_run(
            &input_opts,
            &identifier,
            &run_behavior,
            &session_opts,
            &extension_opts,
            &output_opts,
            &model_opts,
        )
        .await;
    }
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
        ModelsCommand::Decision { command } => match command {
            DecisionModelCommand::List { format } => {
                crate::commands::decide::handle_models_decision_list(&format).await
            }
            DecisionModelCommand::Pull { model } => {
                crate::commands::decision_pull::handle_models_decision_pull(model).await
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

/// Run one `biorouter dictation` subcommand (speech models, devices, the microphone test).
async fn handle_dictation_subcommand(command: DictationCommand) -> Result<()> {
    match command {
        DictationCommand::Status { format, verbose } => {
            crate::commands::dictation::handle_status(&format, verbose).await
        }
        DictationCommand::List { format } => crate::commands::dictation::handle_list(&format).await,
        DictationCommand::Install {
            model,
            from,
            system,
            yes,
            suggest_from_chats,
            no_suggest_from_chats,
        } => {
            crate::commands::dictation::handle_install_with_choice(
                crate::commands::dictation::InstallArgs {
                    model,
                    from,
                    system,
                    yes,
                },
                if suggest_from_chats {
                    Some(true)
                } else if no_suggest_from_chats {
                    Some(false)
                } else {
                    None
                },
            )
            .await
        }
        DictationCommand::Remove { model, system, yes } => {
            crate::commands::dictation::handle_remove(&model, system, yes).await
        }
        DictationCommand::Use { model } => crate::commands::dictation::handle_use(&model).await,
        DictationCommand::Urls { model } => {
            crate::commands::dictation::handle_urls(model.as_deref())
        }
        DictationCommand::Verify {} => crate::commands::dictation::handle_verify().await,
        DictationCommand::Bench { format } => {
            crate::commands::dictation::handle_bench(&format).await
        }
        DictationCommand::WarmUp { model } => {
            crate::commands::dictation::handle_warm_up(&model).await
        }
        DictationCommand::Devices { format } => {
            crate::commands::dictation::handle_devices(&format).await
        }
        DictationCommand::Test { seconds } => {
            crate::commands::dictate::handle_microphone_test(seconds).await
        }
    }
}

/// Run `biorouter dictate`. [`dispatch`] hands over the whole `Command::Dictate` so its
/// table stays one arm per verb; no other variant is routed here.
async fn handle_dictate_command(command: Command) -> Result<()> {
    let Command::Dictate {
        copy,
        no_history,
        format,
        seconds,
        session,
        private_terms,
        language,
        model,
        no_dictionary,
        no_review,
        yes,
        file,
    } = command
    else {
        unreachable!("dispatch routes only `Command::Dictate` here");
    };
    crate::commands::dictate::handle_dictate(crate::commands::dictate::DictateArgs {
        copy,
        no_history,
        format: crate::commands::dictate::DictateFormat::parse(&format),
        seconds,
        session,
        private_terms,
        language,
        model,
        no_dictionary,
        no_review: no_review || yes,
        file,
    })
    .await
}

/// Run `biorouter transcribe`. [`dispatch`] hands over the whole `Command::Transcribe` so
/// its table stays one arm per verb; no other variant is routed here.
async fn handle_transcribe_command(command: Command) -> Result<()> {
    let Command::Transcribe {
        input,
        format,
        output,
        language,
        model,
        session,
        context_file,
        no_dictionary,
    } = command
    else {
        unreachable!("dispatch routes only `Command::Transcribe` here");
    };
    crate::commands::transcribe::handle_transcribe(crate::commands::transcribe::TranscribeArgs {
        input,
        format: crate::commands::transcribe::OutputFormat::parse(&format),
        output,
        language,
        model,
        session,
        context_file,
        no_dictionary,
    })
    .await
}

async fn handle_dictionary_subcommand(command: DictionaryCommand) -> Result<()> {
    crate::commands::dictionary::handle(command.into_action()).await
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
        // A probe that timed out did not disprove the tool: offering to install
        // something the machine may already have is worse than saying nothing.
        deps.iter()
            .find(|d| d.required && !d.installed && !d.timed_out)
            .or_else(|| deps.iter().find(|d| !d.installed && !d.timed_out))
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
                    timed_out: false,
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
    // `biorouter` alone is a new `biorouter session`, and runs where that would (D4).
    let chat = SessionChat::default();
    if let Some(client) = shared_daemon_for(&chat).await {
        return run_default_shared_session(client, chat).await;
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

fn is_shared_conversation(command: &Option<Command>) -> bool {
    match command {
        Some(Command::Run { session_opts, .. } | Command::Session { session_opts, .. }) => {
            session_opts.shared.shared_daemon
        }
        _ => false,
    }
}

fn needs_tool_bridge(command: &Option<Command>) -> bool {
    if is_shared_conversation(command) {
        return false;
    }
    matches!(
        command,
        None | Some(
            Command::Run { .. }
                | Command::Session { command: None, .. }
                | Command::Acp { .. }
                | Command::Doctor { fix: Some(_), .. }
                | Command::Term {
                    command: TermCommand::Run { .. }
                }
                | Command::Bench {
                    cmd: BenchCommand::ExecEval { .. }
                }
                | Command::Schedule {
                    command: SchedulerCommand::RunNow { .. }
                }
        )
    )
}

/// The command line, read and checked.
pub struct CommandLine(Cli);

/// Read the command line. `--help`, `--version` and a usage mistake are answered here and the
/// process exits, so call this before anything writes to disk: a version probe is the first
/// thing an install script runs, and it used to create `config.yaml`, stamp the privacy
/// master-switch record and open a log directory in a home nobody had set up (SF-F8).
pub fn read_command_line() -> CommandLine {
    CommandLine(Cli::parse())
}

pub async fn cli(command_line: CommandLine) -> anyhow::Result<()> {
    let CommandLine(cli) = command_line;

    if !is_shared_conversation(&cli.command) {
        if let Err(e) = crate::project_tracker::update_project_tracker(None, None) {
            warn!("Warning: Failed to update project tracker: {}", e);
        }
    }

    let command_name = get_command_name(&cli.command);
    tracing::info!(
        counter.biorouter.cli_commands = 1,
        command = command_name,
        "CLI command executed"
    );

    let _tool_bridge = if needs_tool_bridge(&cli.command) {
        Some(biorouter::providers::coding_agent::bridge_http::LoopbackBridge::start().await?)
    } else {
        None
    };

    dispatch(cli.command).await
}

/// Mark this process as the terminal surface for the dictation and dictionary code that
/// reads `activation::process_surface` (the first surface set wins). `session` and `run`
/// call it before they start.
fn mark_cli_surface() {
    biorouter::dictation::activation::set_process_surface(
        biorouter::dictation::activation::Surface::Cli,
    );
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
        Some(Command::Crew(options)) => crate::commands::crew::handle(options).await,
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
            local,
            session_opts,
            extension_opts,
            model_opts,
        }) => {
            mark_cli_surface();
            let chat = SessionChat {
                identifier,
                resume,
                history,
                local,
                session_opts,
                extension_opts,
                model_opts,
            };
            handle_session_command(command, chat).await
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
            mark_cli_surface();
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
        Some(Command::Decide(options)) => crate::commands::decide::handle_decide(options).await,
        Some(Command::Models { command }) => handle_models_subcommand(command).await,
        Some(Command::Dictation { command }) => handle_dictation_subcommand(command).await,
        Some(command @ Command::Dictate { .. }) => handle_dictate_command(command).await,
        Some(command @ Command::Transcribe { .. }) => handle_transcribe_command(command).await,
        Some(Command::Dictionary { command }) => handle_dictionary_subcommand(command).await,
        Some(Command::Knowledge { command }) => handle_knowledge_subcommand(command).await,
        Some(Command::Extension { command }) => handle_extension_subcommand(command).await,
        Some(Command::Skill { command }) => handle_skill_subcommand(command).await,
        Some(Command::Apps { command }) => handle_apps_subcommand(command).await,
        Some(Command::Serve(options)) => options.run().await,
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

    /// AG-F8: `--help` is for the people who run the command. A doc comment on a command
    /// becomes its long help unless `long_about` is set, so a maintainer's note there (`⚠`, a
    /// source file, a plan ID, a repository path) reached every `biorouter session --help`.
    /// This walks every command's long help, so a note that leaks again anywhere is caught.
    #[test]
    fn no_command_help_prints_a_maintainers_note() {
        fn walk(command: &mut clap::Command, path: &str, leaks: &mut Vec<String>) {
            let internal =
                regex::Regex::new(r"⚠|\b[\w-]+\.rs\b|\b(?:BR|SD|DR|QA)-\d|\bdocs/|\bcrates/")
                    .expect("the marker pattern compiles");
            let help = command.render_long_help().to_string();
            if let Some(found) = internal.find(&help) {
                leaks.push(format!("{path}: {}", found.as_str()));
            }
            let names: Vec<String> = command
                .get_subcommands()
                .map(|sub| sub.get_name().to_owned())
                .collect();
            for name in names {
                if let Some(sub) = command.find_subcommand_mut(&name) {
                    walk(sub, &format!("{path} {name}"), leaks);
                }
            }
        }
        let mut leaks = Vec::new();
        let mut root = command_tree();
        root.build();
        walk(&mut root, "biorouter", &mut leaks);
        assert!(leaks.is_empty(), "internal notes in --help: {leaks:#?}");
    }

    #[test]
    fn session_subagents_parses_its_id_format_and_watch() {
        use crate::commands::session_subagents::SubagentsFormat;
        let parse = |argv: &[&str]| -> Option<SessionCommand> {
            match Cli::try_parse_from(argv).ok()?.command {
                Some(Command::Session { command, .. }) => command,
                _ => None,
            }
        };

        let Some(SessionCommand::Subagents {
            session_id,
            format,
            watch,
            user_action_key_stdin,
        }) = parse(&["biorouter", "session", "subagents", "20261104_1"])
        else {
            panic!("`session subagents <id>` did not parse as the subagents command");
        };
        assert_eq!(session_id, "20261104_1");
        assert_eq!(format, SubagentsFormat::Text, "text is the default");
        assert!(!watch && !user_action_key_stdin);

        let Some(SessionCommand::Subagents { format, watch, .. }) = parse(&[
            "biorouter",
            "session",
            "subagents",
            "20261104_1",
            "--format",
            "json",
            "--watch",
        ]) else {
            panic!("`--format json --watch` did not parse");
        };
        assert_eq!(format, SubagentsFormat::Json);
        assert!(watch);

        let Some(SessionCommand::Subagents {
            user_action_key_stdin,
            ..
        }) = parse(&[
            "biorouter",
            "session",
            "subagents",
            "20261104_1",
            "-f",
            "text",
            "--user-action-key-stdin",
        ])
        else {
            panic!("`-f text --user-action-key-stdin` did not parse");
        };
        assert!(user_action_key_stdin);

        // The id is required, and a format it does not know is refused rather
        // than read as text.
        assert!(Cli::try_parse_from(["biorouter", "session", "subagents"]).is_err());
        assert!(Cli::try_parse_from([
            "biorouter",
            "session",
            "subagents",
            "20261104_1",
            "--format",
            "yaml"
        ])
        .is_err());
    }

    #[test]
    fn decide_parses_as_a_top_level_command() {
        let parsed = Cli::try_parse_from([
            "biorouter",
            "decide",
            "--model",
            "ollama_decision/tev1:0.8b",
            "--ask",
            "Is an EDSS score stated?",
            "--threshold",
            "0.8",
            "--json",
        ])
        .expect("decide parses");
        let Some(Command::Decide(options)) = parsed.command else {
            panic!("not the decide command");
        };
        assert_eq!(options.model.as_deref(), Some("ollama_decision/tev1:0.8b"));
        assert_eq!(options.ask, "Is an EDSS score stated?");
        assert_eq!(options.threshold, Some(0.8));
        assert!(options.json && options.session.is_none() && options.batch.is_none());
        assert_eq!(get_command_name(&Some(Command::Decide(options))), "decide");
        assert!(
            Cli::try_parse_from(["biorouter", "decide"]).is_err(),
            "--ask is required"
        );
    }

    #[test]
    fn models_decision_list_parses() {
        let parsed = Cli::try_parse_from([
            "biorouter",
            "models",
            "decision",
            "list",
            "--format",
            "json",
        ])
        .expect("models decision list parses");
        let Some(Command::Models {
            command:
                ModelsCommand::Decision {
                    command: DecisionModelCommand::List { format },
                },
        }) = parsed.command
        else {
            panic!("not models decision list");
        };
        assert_eq!(format, "json");
        assert!(Cli::try_parse_from(["biorouter", "models", "decision", "ls"]).is_ok());

        // `pull` installs the default unless a model is named.
        let pull = |argv: &[&str]| -> Option<Option<String>> {
            match Cli::try_parse_from(argv).ok()?.command {
                Some(Command::Models {
                    command:
                        ModelsCommand::Decision {
                            command: DecisionModelCommand::Pull { model },
                        },
                }) => Some(model),
                _ => None,
            }
        };
        assert_eq!(
            pull(&["biorouter", "models", "decision", "pull"]),
            Some(None)
        );
        assert_eq!(
            pull(&[
                "biorouter",
                "models",
                "decision",
                "pull",
                "clef-flash:latest"
            ]),
            Some(Some("clef-flash:latest".to_string()))
        );
        assert_eq!(
            crate::commands::decision_pull::requested_model(None),
            "tev1:0.8b"
        );
        assert!(Cli::try_parse_from([
            "biorouter",
            "models",
            "decision",
            "list",
            "--format",
            "yaml"
        ])
        .is_err());
    }

    #[test]
    fn session_decisions_and_monitors_parse() {
        use crate::commands::session_decisions::{OutcomeArg, SourceArg};
        use crate::commands::session_monitors::MonitorsCommand;
        let parse = |argv: &[&str]| -> Option<SessionCommand> {
            match Cli::try_parse_from(argv).ok()?.command {
                Some(Command::Session { command, .. }) => command,
                _ => None,
            }
        };

        let Some(SessionCommand::Decisions {
            session_id,
            filters,
            json,
            user_action_key_stdin,
        }) = parse(&[
            "biorouter",
            "session",
            "decisions",
            "20261104_1",
            "--source",
            "tool",
            "--turn",
            "t1",
            "--outcome",
            "error",
            "--errors",
            "-l",
            "50",
            "--json",
        ])
        else {
            panic!("`session decisions` did not parse");
        };
        assert_eq!(session_id, "20261104_1");
        assert_eq!(filters.source, Some(SourceArg::Tool));
        assert_eq!(filters.turn.as_deref(), Some("t1"));
        assert_eq!(filters.outcome, Some(OutcomeArg::Error));
        assert!(filters.errors && json && !user_action_key_stdin);
        assert_eq!(filters.limit, 50);

        let Some(SessionCommand::Decisions { filters, .. }) =
            parse(&["biorouter", "session", "decisions", "20261104_1"])
        else {
            panic!("bare `session decisions <id>` did not parse");
        };
        assert_eq!(
            filters.limit,
            crate::commands::session_decisions::DEFAULT_LIMIT
        );
        assert!(parse(&["biorouter", "session", "decisions"]).is_none());
        assert!(parse(&[
            "biorouter",
            "session",
            "decisions",
            "s1",
            "--source",
            "everything"
        ])
        .is_none());

        let Some(SessionCommand::Monitors {
            session_id,
            command: MonitorsCommand::Add(add),
            user_action_key_stdin,
        }) = parse(&[
            "biorouter",
            "session",
            "monitors",
            "20261104_1",
            "add",
            "--name",
            "Form reached",
            "--ask",
            "Has the agent reached the consent form?",
            "--threshold",
            "0.85",
            "--action",
            "stop",
            "--user-action-key-stdin",
        ])
        else {
            panic!("`session monitors <id> add` did not parse");
        };
        assert_eq!(session_id, "20261104_1");
        assert_eq!(add.name, "Form reached");
        assert!(
            user_action_key_stdin,
            "the key flag is global to the subcommands"
        );

        assert!(matches!(
            parse(&["biorouter", "session", "monitors", "s1", "list"]),
            Some(SessionCommand::Monitors {
                command: MonitorsCommand::List { json: false },
                ..
            })
        ));
        assert!(matches!(
            parse(&[
                "biorouter",
                "session",
                "monitors",
                "s1",
                "remove",
                "form-reached"
            ]),
            Some(SessionCommand::Monitors {
                command: MonitorsCommand::Remove { .. },
                ..
            })
        ));
        assert!(parse(&["biorouter", "session", "monitors", "s1"]).is_none());
    }

    #[test]
    fn session_gates_parses_with_and_without_a_subcommand() {
        use crate::commands::session_gates::{GatesCommand, GatesFormat};
        let parse = |argv: &[&str]| -> Option<SessionCommand> {
            match Cli::try_parse_from(argv).ok()?.command {
                Some(Command::Session { command, .. }) => command,
                _ => None,
            }
        };
        // Bare, it lists, in text.
        let Some(SessionCommand::Gates {
            session_id,
            command: None,
            format: GatesFormat::Text,
            user_action_key_stdin: false,
        }) = parse(&["biorouter", "session", "gates", "20261106_1"])
        else {
            panic!("bare `session gates <id>` did not parse");
        };
        assert_eq!(session_id, "20261106_1");
        // `--format` and the key flag are global to the subcommands.
        assert!(matches!(
            parse(&[
                "biorouter",
                "session",
                "gates",
                "s1",
                "list",
                "--format",
                "json"
            ]),
            Some(SessionCommand::Gates {
                command: Some(GatesCommand::List),
                format: GatesFormat::Json,
                ..
            })
        ));
        assert!(matches!(
            parse(&["biorouter", "session", "gates", "s1", "--format", "json"]),
            Some(SessionCommand::Gates {
                command: None,
                format: GatesFormat::Json,
                ..
            })
        ));
        let Some(SessionCommand::Gates {
            command: Some(GatesCommand::Set(set)),
            user_action_key_stdin: true,
            ..
        }) = parse(&[
            "biorouter",
            "session",
            "gates",
            "s1",
            "set",
            "research",
            "--ask",
            "facts=Does the hand-off list two facts about the moon?",
            "--threshold",
            "0.3",
            "--user-action-key-stdin",
        ])
        else {
            panic!("`session gates <id> set` did not parse");
        };
        assert_eq!(set.phase, "research");
        assert_eq!(set.threshold, Some(0.3));
        for word in ["remove", "pass", "skip", "retry"] {
            assert!(
                parse(&["biorouter", "session", "gates", "s1", word, "research"]).is_some(),
                "{word}"
            );
        }
        assert!(
            parse(&["biorouter", "session", "gates"]).is_none(),
            "an id is needed"
        );
    }

    #[test]
    fn shared_daemon_options_parse_on_session_and_run_without_starting_tool_bridge() {
        let session = Cli::try_parse_from([
            "biorouter",
            "session",
            "--shared-daemon",
            "--approval-key-stdin",
            "--no-start",
            "--create-only",
            "--provider",
            "synthetic-provider",
            "--model",
            "synthetic-model",
        ])
        .expect("shared session options parse");
        assert!(!needs_tool_bridge(&session.command));
        let Some(Command::Session { session_opts, .. }) = session.command else {
            panic!("session command expected");
        };
        assert!(session_opts.shared.shared_daemon);
        assert!(session_opts.shared.approval_key_stdin);
        assert!(session_opts.shared.no_start);
        assert!(session_opts.shared.create_only);

        let run = Cli::try_parse_from([
            "biorouter",
            "run",
            "--shared-daemon",
            "--text",
            "synthetic prompt",
            "--provider",
            "synthetic-provider",
            "--model",
            "synthetic-model",
        ])
        .expect("shared run options parse");
        assert!(!needs_tool_bridge(&run.command));
        assert!(matches!(run.command, Some(Command::Run { .. })));
    }

    #[test]
    fn shared_daemon_dependent_flags_are_rejected_without_the_shared_switch() {
        for flag in ["--approval-key-stdin", "--no-start", "--create-only"] {
            let error = Cli::try_parse_from(["biorouter", "run", flag])
                .err()
                .expect("dependent shared flag must require --shared-daemon");
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::MissingRequiredArgument
            );
        }
    }

    #[test]
    fn shared_conversation_resolution_requires_exact_resume_ids_and_new_provider_model() {
        let shared = SessionOptions {
            shared: SharedDaemonOptions {
                shared_daemon: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let extensions = ExtensionOptions::default();

        let resumed = shared_conversation_options(
            &Some(Identifier {
                session_id: Some("session-123".into()),
                name: None,
                path: None,
            }),
            true,
            &shared,
            &extensions,
            &ModelOptions::default(),
        )
        .expect("exact session IDs resume without local lookup");
        assert_eq!(resumed.session_id.as_deref(), Some("session-123"));
        assert!(!resumed.create_only);

        let new = shared_conversation_options(
            &None,
            false,
            &shared,
            &extensions,
            &ModelOptions {
                provider: Some("synthetic-provider".into()),
                model: Some("synthetic-model".into()),
            },
        )
        .expect("new shared conversations accept explicit provider and model");
        assert_eq!(new.provider.as_deref(), Some("synthetic-provider"));
        assert_eq!(new.model.as_deref(), Some("synthetic-model"));

        let error = shared_conversation_options(
            &Some(Identifier {
                session_id: Some("session-123".into()),
                name: None,
                path: None,
            }),
            false,
            &shared,
            &extensions,
            &ModelOptions::default(),
        )
        .err()
        .expect("an exact session ID without --resume must be refused");
        assert!(error.to_string().contains("requires --resume"));

        let error = shared_conversation_options(
            &None,
            false,
            &shared,
            &extensions,
            &ModelOptions::default(),
        )
        .err()
        .expect("new shared conversations must name provider and model");
        assert!(error
            .to_string()
            .contains("requires both --provider and --model"));
    }

    /// The `biorouter session` chat a command line describes.
    fn session_chat(argv: &[&str]) -> SessionChat {
        let Some(Command::Session {
            command: None,
            identifier,
            resume,
            history,
            local,
            session_opts,
            extension_opts,
            model_opts,
        }) = Cli::try_parse_from(argv).expect("parses").command
        else {
            panic!("an interactive session expected: {argv:?}");
        };
        SessionChat {
            identifier,
            resume,
            history,
            local,
            session_opts,
            extension_opts,
            model_opts,
        }
    }

    /// D4: `--local` is a `biorouter session` option only, and cannot be combined with
    /// `--shared-daemon`; `biorouter run` stays in this process and has no such option.
    #[test]
    fn local_is_a_session_option_that_excludes_the_shared_daemon() {
        assert!(session_chat(&["biorouter", "session", "--local"]).local);
        assert!(!session_chat(&["biorouter", "session"]).local);
        let both = Cli::try_parse_from(["biorouter", "session", "--local", "--shared-daemon"])
            .err()
            .expect("--local and --shared-daemon conflict");
        assert_eq!(both.kind(), clap::error::ErrorKind::ArgumentConflict);
        let run = Cli::try_parse_from(["biorouter", "run", "--local", "--text", "hi"])
            .err()
            .expect("run has no --local");
        assert_eq!(run.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    /// D4: what decides where a chat runs, read from its command line. An option only a chat in
    /// this process takes keeps it here, and so does `--session-id` without `--resume`; a chat
    /// resumed by its ID needs both model flags or neither.
    #[test]
    fn a_session_command_line_says_where_its_chat_may_run() {
        let request = |argv: &[&str]| chat_request(&session_chat(argv));
        let plain = request(&["biorouter", "session"]);
        assert!(!plain.local && !plain.in_process_option);
        assert!(request(&["biorouter", "session", "--local"]).local);
        for option in [
            &["--debug"][..],
            &["--name", "notes"],
            &["--max-turns", "3"],
            &["--max-tool-repetitions", "2"],
            &["--with-builtin", "developer"],
            &["--with-extension", "echo"],
            &["--with-streamable-http-extension", "http://127.0.0.1:9/mcp"],
            &["--session-id", "20261107_1"],
        ] {
            let argv: Vec<&str> = ["biorouter", "session"]
                .iter()
                .chain(option)
                .copied()
                .collect();
            assert!(request(&argv).in_process_option, "{argv:?}");
        }
        let resumed = request(&[
            "biorouter",
            "session",
            "--resume",
            "--session-id",
            "20261107_1",
        ]);
        assert!(!resumed.in_process_option && resumed.model_known);
        let rebound = [
            "biorouter",
            "session",
            "--resume",
            "--session-id",
            "20261107_1",
            "--provider",
            "p",
            "--model",
            "m",
        ];
        assert!(request(&rebound).model_known);
        let half = [
            "biorouter",
            "session",
            "--resume",
            "--session-id",
            "20261107_1",
            "--provider",
            "p",
        ];
        assert!(!request(&half).model_known);
        assert!(request(&["biorouter", "session", "--provider", "p", "--model", "m"]).model_known);
        assert!(!request(&["biorouter", "session", "--provider", "p"]).model_known);
    }

    /// `biorouter session --shared-daemon` with neither model flag starts a new interactive
    /// chat on the configured pair; `--create-only`, a named chat or a given flag keep what was
    /// given, so the conversation still refuses a pair that is incomplete.
    #[test]
    fn a_new_shared_daemon_chat_takes_the_configured_model() {
        let configured = || {
            (
                Some("cfg-provider".to_owned()),
                Some("cfg-model".to_owned()),
            )
        };
        let filled = new_chat_model(ModelOptions::default(), true, configured);
        assert_eq!(filled.provider.as_deref(), Some("cfg-provider"));
        assert_eq!(filled.model.as_deref(), Some("cfg-model"));
        let kept = new_chat_model(ModelOptions::default(), false, configured);
        assert!(kept.provider.is_none() && kept.model.is_none());
        let named = ModelOptions {
            provider: Some("p".into()),
            model: None,
        };
        let kept = new_chat_model(named, true, configured);
        assert_eq!(kept.provider.as_deref(), Some("p"));
        assert!(kept.model.is_none());
        let unconfigured = new_chat_model(ModelOptions::default(), true, || (None, None));
        assert!(unconfigured.provider.is_none() && unconfigured.model.is_none());
    }

    /// Only a new interactive chat at a terminal that names its model is started by this
    /// command (with the configured extensions); every other shape goes to the conversation as
    /// given, which refuses it before creating anything.
    #[test]
    fn only_a_new_interactive_chat_at_a_terminal_is_started_here() {
        use crate::commands::shared_conversation::SharedConversationOptions;
        fn new_chat() -> SharedConversationOptions {
            SharedConversationOptions {
                session_id: None,
                working_dir: PathBuf::from("/tmp"),
                provider: Some("p".into()),
                model: Some("m".into()),
                prompt: None,
                interactive: true,
                create_only: false,
                history: false,
                approval_key_stdin: false,
                no_start: true,
                quiet: false,
                output_format: "text".into(),
            }
        }
        let with = |change: fn(&mut SharedConversationOptions)| {
            let mut options = new_chat();
            change(&mut options);
            starts_here(&options, true)
        };
        assert_eq!(
            starts_here(&new_chat(), true),
            Some(("p".into(), "m".into()))
        );
        assert_eq!(starts_here(&new_chat(), false), None, "no terminal");
        assert_eq!(
            with(|o| o.session_id = Some("20261107_1".into())),
            None,
            "resumed"
        );
        assert_eq!(
            with(|o| o.interactive = false),
            None,
            "create-only or one prompt"
        );
        assert_eq!(
            with(|o| o.approval_key_stdin = true),
            None,
            "stdin is taken"
        );
        assert_eq!(with(|o| o.model = None), None, "no model");
    }

    #[tokio::test]
    async fn shared_run_rejects_before_reading_an_instruction_file_or_building_a_provider() {
        let input = InputOptions {
            instructions: Some("/path/that/must/not/be/read".into()),
            ..Default::default()
        };
        let behavior = RunBehavior::default();
        let session = SessionOptions {
            shared: SharedDaemonOptions {
                shared_daemon: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let error = handle_shared_run(
            &input,
            &None,
            &behavior,
            &session,
            &ExtensionOptions::default(),
            &OutputOptions::default(),
            &ModelOptions::default(),
        )
        .await
        .expect_err("provider/model refusal must precede instruction-file reading");
        assert!(error
            .to_string()
            .contains("requires both --provider and --model"));
        assert!(!error.to_string().contains("No such file"));
    }

    #[test]
    fn every_standalone_agent_entry_point_starts_the_tool_bridge() {
        for args in [
            vec!["biorouter"],
            vec!["biorouter", "run"],
            vec!["biorouter", "session"],
            vec!["biorouter", "acp"],
            vec!["biorouter", "doctor", "--fix", "node"],
            vec!["biorouter", "term", "run", "synthetic prompt"],
            vec!["biorouter", "bench", "exec-eval", "--config", "synthetic"],
            vec![
                "biorouter",
                "schedule",
                "run-now",
                "--schedule-id",
                "synthetic",
            ],
        ] {
            let parsed = Cli::try_parse_from(&args).unwrap();
            assert!(needs_tool_bridge(&parsed.command), "{args:?}");
        }
        for args in [
            vec!["biorouter", "info"],
            vec!["biorouter", "doctor"],
            vec!["biorouter", "session", "list"],
            vec!["biorouter", "schedule", "list"],
            vec!["biorouter", "term", "init", "zsh"],
        ] {
            let parsed = Cli::try_parse_from(&args).unwrap();
            assert!(!needs_tool_bridge(&parsed.command), "{args:?}");
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn cli_knowledge_startup_purges_legacy_bases_and_preserves_current_profiles() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
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
        let _env = crate::test_sandbox::relocate_path_root_and(
            tmp.path(),
            [("BIOROUTER_DISABLE_KEYRING", Some("1".to_string()))],
        );
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
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
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
        let _env = crate::test_sandbox::relocate_path_root_and(
            tmp.path(),
            [("BIOROUTER_DISABLE_KEYRING", Some("1".to_string()))],
        );
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
            .split_once("\nasync fn refusal_for_a_new_row(")
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
    #[tokio::test]
    async fn a_configured_default_satisfies_the_precondition_for_a_new_row() {
        let _env = env_lock::lock_env([
            ("BIOROUTER_PROVIDER", Some("claude_code".to_string())),
            ("BIOROUTER_MODEL", Some("claude-opus-5".to_string())),
        ]);

        assert_eq!(
            resolution_for_a_new_row(None, None).await,
            (
                Some("claude_code".to_string()),
                Some("claude-opus-5".to_string())
            ),
            "a bare `biorouter` names no provider, so the configured one is the answer"
        );
        assert_eq!(
            refusal_for_a_new_row(None, None).await,
            None,
            "a configured install must get past the guard with no flags at all"
        );
    }

    /// What the user typed now still wins over what they configured earlier.
    #[tokio::test]
    async fn an_explicit_flag_still_outranks_the_configured_default() {
        let _env = env_lock::lock_env([
            ("BIOROUTER_PROVIDER", Some("claude_code".to_string())),
            ("BIOROUTER_MODEL", Some("claude-opus-5".to_string())),
        ]);
        assert_eq!(
            resolution_for_a_new_row(Some("anthropic"), Some("opus")).await,
            (Some("anthropic".to_string()), Some("opus".to_string()))
        );
    }

    /// The model has the same four slots as the provider, and shipped with the
    /// same gap: `biorouter run --provider claude_code` got past the provider
    /// half and was refused with "No model is configured".
    ///
    /// PROV-F10: naming another provider must not cost a model, and must not
    /// borrow the configured provider's either: it runs its own default. Only a
    /// provider the registry cannot describe falls back to the configured model.
    #[test]
    fn the_model_slot_falls_back_too() {
        assert_eq!(
            resolution_with_global_default(
                Some("claude_code"),
                None,
                Some("versa_azure".to_string()),
                Some("gpt-5.5".to_string()),
                Some("claude-sonnet-5".to_string()),
            ),
            (
                Some("claude_code".to_string()),
                Some("claude-sonnet-5".to_string())
            ),
            "another provider runs its own default model, not the configured provider's"
        );
        assert_eq!(
            resolution_with_global_default(
                Some("claude_code"),
                None,
                Some("versa_azure".to_string()),
                Some("gpt-5.5".to_string()),
                None,
            ),
            (Some("claude_code".to_string()), Some("gpt-5.5".to_string())),
            "a provider with no known default still gets past the guard"
        );
        assert_eq!(
            resolution_with_global_default(
                Some("versa_azure"),
                None,
                Some("versa_azure".to_string()),
                Some("gpt-5.5".to_string()),
                Some("gpt-6-sol".to_string()),
            ),
            (Some("versa_azure".to_string()), Some("gpt-5.5".to_string())),
            "naming the configured provider keeps the configured model"
        );
    }

    /// PROV-F10, through the real `Config` and registry: `--provider codex` with a
    /// Versa model configured resolves Codex's own default model.
    #[tokio::test]
    async fn naming_another_provider_resolves_its_own_default_model() {
        let _env = env_lock::lock_env([
            ("BIOROUTER_PROVIDER", Some("versa_azure".to_string())),
            ("BIOROUTER_MODEL", Some("gpt-5.5-2026-04-24".to_string())),
        ]);
        let (provider, model) = resolution_for_a_new_row(Some("codex"), None).await;
        assert_eq!(provider.as_deref(), Some("codex"));
        let model = model.expect("codex has a default model");
        assert_ne!(
            model, "gpt-5.5-2026-04-24",
            "the configured Versa model was sent to Codex"
        );
    }

    /// The behaviour `5dc1eee1` added, which this fix must not undo: an install
    /// with nothing configured anywhere is still refused, and the refusal names
    /// the provider first.
    #[test]
    fn an_install_with_nothing_configured_is_still_refused() {
        assert_eq!(
            resolution_with_global_default(None, None, None, None, None),
            (None, None)
        );
        let text = crate::session::unconfigured_precondition(None, None)
            .expect("nothing configured anywhere must still be refused");
        assert!(text.contains("No provider is configured"), "{text}");

        // A workflow pin alone is enough — that is the whole reason `run`
        // resolves one before asking.
        assert_eq!(
            resolution_with_global_default(
                Some("claude_code"),
                Some("claude-opus-5"),
                None,
                None,
                None
            ),
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

    #[test]
    fn serve_options_preserve_defaults_alias_flags_and_token_conflict() {
        let parsed = Cli::try_parse_from(["biorouter", "serve"]).unwrap();
        let Some(Command::Serve(options)) = parsed.command else {
            panic!("serve command expected");
        };
        assert_eq!(options.host, "127.0.0.1");
        assert_eq!(options.port, crate::commands::serve::DEFAULT_PORT);
        assert!(options.token.is_none());
        assert!(options.web_dir.is_none());
        assert!(!options.no_token);
        assert!(!options.no_dictation_install);
        assert!(!options.computer_use_approval);
        assert!(!options.folder_access_approval);
        assert!(!options.open);

        let parsed = Cli::try_parse_from([
            "biorouter",
            "serve",
            "--folder-access-approval",
            "--computer-use-approval",
        ])
        .unwrap();
        let Some(Command::Serve(options)) = parsed.command else {
            panic!("serve command expected")
        };
        assert!(options.folder_access_approval && options.computer_use_approval);

        let parsed = Cli::try_parse_from([
            "biorouter",
            "headless",
            "--host",
            "0.0.0.0",
            "-p",
            "9876",
            "--token",
            "test-token",
            "--web-dir",
            "/tmp/web",
            "--computer-use-approval",
            "--open",
        ])
        .unwrap();
        assert_eq!(get_command_name(&parsed.command), "serve");
        let Some(Command::Serve(options)) = parsed.command else {
            panic!("headless alias must select serve");
        };
        assert_eq!(options.host, "0.0.0.0");
        assert_eq!(options.port, 9876);
        assert_eq!(options.token.as_deref(), Some("test-token"));
        assert_eq!(
            options.web_dir.as_deref(),
            Some(std::path::Path::new("/tmp/web"))
        );
        assert!(!options.no_token);
        assert!(!options.no_dictation_install);
        assert!(options.computer_use_approval);
        assert!(options.open);

        let parsed = Cli::try_parse_from([
            "biorouter",
            "serve",
            "--no-token",
            "--computer-use-approval",
            "--open",
        ])
        .unwrap();
        let Some(Command::Serve(options)) = parsed.command else {
            panic!("serve command expected");
        };
        assert!(options.no_token && options.computer_use_approval && options.open);

        let parsed = Cli::try_parse_from(["biorouter", "serve", "--no-dictation-install"]).unwrap();
        let Some(Command::Serve(options)) = parsed.command else {
            panic!("serve command expected");
        };
        assert!(options.no_dictation_install);

        let conflict =
            Cli::try_parse_from(["biorouter", "serve", "--token", "test-token", "--no-token"])
                .err()
                .expect("token and no-token must conflict");
        assert_eq!(conflict.kind(), clap::error::ErrorKind::ArgumentConflict);
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

#[cfg(test)]
mod default_max_turns_tests {
    use biorouter::agents::DEFAULT_MAX_TURNS;

    /// The `--max-turns` help text announced `(default: 1000)` while the real
    /// default was 100, so the CLI told every user a number ten times the one it
    /// would use. The number has to appear as a literal (clap's `help` takes a
    /// `&'static str`), so this is what keeps the literal honest.
    #[test]
    fn the_help_text_announces_the_default_the_agent_actually_uses() {
        let source = include_str!("cli.rs");
        let line = source
            .lines()
            .find(|line| line.contains("Maximum number of turns allowed without user input"))
            .expect("the --max-turns help string moved; update this test with it");
        assert!(
            line.contains(&format!("(default: {DEFAULT_MAX_TURNS}, unlimited)")),
            "--max-turns help announces a different default from DEFAULT_MAX_TURNS ({DEFAULT_MAX_TURNS}): {line}"
        );
    }

    /// `biorouter configure` pre-filled 1000 as the "current" value when nothing
    /// was set, so a user who opened the dialog and pressed Enter silently wrote
    /// 1000 into their config and raised the limit tenfold. The dialog must read
    /// the constant, never restate a number.
    #[test]
    fn the_configure_dialog_falls_back_to_the_real_default() {
        let source = include_str!("commands/configure.rs");
        let fallback = source
            .split("BIOROUTER_MAX_TURNS")
            .nth(1)
            .expect("the max-turns configure dialog moved; update this test with it");
        assert!(
            fallback.contains("DEFAULT_MAX_TURNS"),
            "the configure dialog's max-turns fallback restates a literal instead of reading \
             DEFAULT_MAX_TURNS; it read 1000 while the agent used 100"
        );
    }
}

#[cfg(test)]
mod local_model_help_tests {
    use super::Cli;
    use biorouter::providers::llamacpp::{MODEL_CATALOG, RETIRED_MODELS};
    use clap::CommandFactory;

    fn model_arg_help(subcommand: &str) -> String {
        let mut command = Cli::command();
        let local = command
            .find_subcommand_mut("models")
            .and_then(|models| models.find_subcommand_mut("local"))
            .expect("`biorouter models local` exists");
        let sub = local
            .find_subcommand_mut(subcommand)
            .unwrap_or_else(|| panic!("`biorouter models local {subcommand}` exists"));
        let help = sub
            .get_arguments()
            .find(|arg| arg.get_id() == "model")
            .and_then(|arg| arg.get_help())
            .map(|help| help.to_string());
        help.unwrap_or_else(|| panic!("`{subcommand}` has a model argument with help"))
    }

    /// `pull`'s help named `qwen3.5-4b` through two catalog changes: an example
    /// that, followed, downloaded a model no chat would run. It must name only
    /// models the catalog offers, and the clap `help` string cannot read the
    /// catalog itself, so this keeps the literal honest.
    #[test]
    fn pull_help_names_the_catalog_and_no_retired_model() {
        let help = model_arg_help("pull");
        for entry in MODEL_CATALOG {
            assert!(help.contains(entry.name), "{help}");
        }
        for retired in RETIRED_MODELS {
            let named = help
                .split(|c: char| c.is_whitespace() || "(),;".contains(c))
                .any(|word| word == retired.name);
            assert!(!named, "pull help names retired {}: {help}", retired.name);
        }
    }

    /// `rm` may name a retired model (deleting its files is what it is for), but
    /// its first example must be a model the catalog offers.
    #[test]
    fn rm_help_leads_with_a_catalog_model() {
        let help = model_arg_help("rm");
        let first_catalog_name = MODEL_CATALOG
            .iter()
            .filter_map(|entry| help.find(entry.name))
            .min()
            .expect("rm help names a catalog model");
        let first_retired_name = RETIRED_MODELS
            .iter()
            .filter_map(|retired| help.find(&format!("e.g. {}", retired.name)))
            .min();
        assert!(
            first_retired_name.is_none_or(|at| first_catalog_name < at),
            "{help}"
        );
    }
}
