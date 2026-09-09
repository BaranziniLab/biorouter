use super::output;
use super::privacy::{available_private_models, repair_block, terminal_refusal, ProviderSource};
use super::CliSession;
use biorouter::agents::{Agent, AgentConfig};
use biorouter::config::get_enabled_extensions;
use biorouter::config::resolve_extensions_for_new_session;
use biorouter::config::{
    extensions::get_extension_by_name, get_all_extensions, BioRouterMode, Config, ExtensionConfig,
    PermissionManager,
};
use biorouter::providers::create;
use biorouter::session::session_manager::SessionType;
use biorouter::session::{EnabledExtensionsState, ExtensionState, SessionManager};
use biorouter::workflow::Workflow;
use console::style;
use rustyline::EditMode;
use std::collections::BTreeSet;
use std::path::Path;
use std::process;
use std::sync::Arc;
use tokio::task::JoinSet;

const EXTENSION_HINT_MAX_LEN: usize = 5;

/// Issue #56 Task 31 / R10 — the refusal a chat earns before it starts, or
/// `None` if it may start.
///
/// The four precedence slots do not all deserve the same sentence. Three of them
/// are settings on *this* machine; the fourth is a pin inside a workflow YAML
/// somebody else authored and mailed around, and that one is checked by the
/// workflow module's own load-time check so the refusal names the workflow
/// rather than the model. Both endings share [`repair_block`], because there is
/// only one repair.
///
/// DR-15's master switch is read by both branches — directly, because a session
/// start is not a tool call and has no [`biorouter::privacy::CallCapability`] to
/// inherit a sample from.
async fn privacy_start_refusal(
    provider_name: &str,
    source: ProviderSource,
    classification: biorouter::privacy::SessionClassification,
    session_id: Option<&str>,
    workflow_settings: Option<&biorouter::workflow::Settings>,
) -> Option<String> {
    // The id is only ever interpolated into the two repair commands. A
    // `--no-session` run has none, and cannot reach here anyway: its row is born
    // Public.
    let session_id = session_id.unwrap_or("<session-id>");

    if source == ProviderSource::Workflow {
        let why = biorouter::workflow::privacy::assert_workflow_provider_allowed(
            workflow_settings,
            classification,
        )
        .await
        .err()?;
        return Some(format!(
            "{why}\n{}",
            repair_block(session_id, &available_private_models().await)
        ));
    }

    if !biorouter::privacy::privacy_tiers_enabled() {
        return None;
    }
    let tier = biorouter::workflow::privacy::declared_provider_tier(provider_name).await;
    if biorouter::privacy::bind_allowed(tier, classification) {
        return None;
    }
    Some(terminal_refusal(
        session_id,
        provider_name,
        source,
        &available_private_models().await,
    ))
}

fn truncate_with_ellipsis(s: &str, max_len: usize) -> String {
    let truncated: String = s.chars().take(max_len).collect();
    if s.chars().count() > max_len {
        format!("{}…", truncated)
    } else {
        truncated
    }
}

fn parse_cli_flag_extensions(
    extensions: &[String],
    streamable_http_extensions: &[String],
    builtins: &[String],
) -> Vec<(String, ExtensionConfig)> {
    let mut extensions_to_load = Vec::new();

    for (idx, ext_str) in extensions.iter().enumerate() {
        match CliSession::parse_stdio_extension(ext_str) {
            Ok(config) => {
                let hint = truncate_with_ellipsis(ext_str, EXTENSION_HINT_MAX_LEN);
                let label = format!("stdio #{}({})", idx + 1, hint);
                extensions_to_load.push((label, config));
            }
            Err(e) => {
                eprintln!(
                    "{}",
                    style(format!(
                        "Warning: Invalid --extension value '{}' ({}); ignoring",
                        ext_str, e
                    ))
                    .yellow()
                );
            }
        }
    }

    for (idx, ext_str) in streamable_http_extensions.iter().enumerate() {
        let config = CliSession::parse_streamable_http_extension(ext_str);
        let hint = truncate_with_ellipsis(ext_str, EXTENSION_HINT_MAX_LEN);
        let label = format!("http #{}({})", idx + 1, hint);
        extensions_to_load.push((label, config));
    }

    for builtin_str in builtins {
        let configs = CliSession::parse_builtin_extensions(builtin_str);
        for config in configs {
            extensions_to_load.push((config.name(), config));
        }
    }

    extensions_to_load
}

/// Configuration for building a new Biorouter session
///
/// This struct contains all the parameters needed to create a new session,
/// including session identification, extension configuration, and debug settings.
#[derive(Clone, Debug)]
pub struct SessionBuilderConfig {
    /// Session id, optional need to deduce from context
    pub session_id: Option<String>,
    /// Whether to resume an existing session
    pub resume: bool,
    /// Whether to run without a session file
    pub no_session: bool,
    /// List of stdio extension commands to add
    pub extensions: Vec<String>,
    /// List of streamable HTTP extension commands to add
    pub streamable_http_extensions: Vec<String>,
    /// List of builtin extension commands to add
    pub builtins: Vec<String>,
    /// Workflow for the session
    pub workflow: Option<Workflow>,
    /// Any additional system prompt to append to the default
    pub additional_system_prompt: Option<String>,
    /// Provider override from CLI arguments
    pub provider: Option<String>,
    /// Model override from CLI arguments
    pub model: Option<String>,
    /// Enable debug printing
    pub debug: bool,
    /// Maximum number of consecutive identical tool calls allowed
    pub max_tool_repetitions: Option<u32>,
    /// Maximum number of turns (iterations) allowed without user input
    pub max_turns: Option<u32>,
    /// ID of the scheduled job that triggered this session (if any)
    pub scheduled_job_id: Option<String>,
    /// Whether this session will be used interactively (affects debugging prompts)
    pub interactive: bool,
    /// Quiet mode - suppress non-response output
    pub quiet: bool,
    /// Output format (text, json)
    pub output_format: String,
}

/// Manual implementation of Default to ensure proper initialization of output_format
/// This struct requires explicit default value for output_format field
impl Default for SessionBuilderConfig {
    fn default() -> Self {
        SessionBuilderConfig {
            session_id: None,
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
            interactive: false,
            quiet: false,
            output_format: "text".to_string(),
        }
    }
}

/// Offers to help debug an extension failure by creating a minimal debugging session
async fn offer_extension_debugging_help(
    extension_name: &str,
    error_message: &str,
    provider: Arc<dyn biorouter::providers::base::Provider>,
    interactive: bool,
) -> Result<(), anyhow::Error> {
    // Only offer debugging help in interactive mode
    if !interactive {
        return Ok(());
    }

    let help_prompt = format!(
        "Would you like me to help debug the '{}' extension failure?",
        extension_name
    );

    let should_help = match cliclack::confirm(help_prompt)
        .initial_value(false)
        .interact()
    {
        Ok(choice) => choice,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::Interrupted {
                return Ok(());
            } else {
                return Err(e.into());
            }
        }
    };

    if !should_help {
        return Ok(());
    }

    println!("{}", style("Starting debugging chat...").cyan());

    // Create a debugging prompt with context about the extension failure
    let debug_prompt = format!(
        "I'm having trouble starting an extension called '{}'. Here's the error I encountered:\n\n{}\n\nCan you help me diagnose what might be wrong and suggest how to fix it? Please consider common issues like:\n- Missing dependencies or tools\n- Configuration problems\n- Network connectivity (for remote extensions)\n- Permission issues\n- Path or environment variable problems",
        extension_name,
        error_message
    );

    // Create a minimal agent for debugging
    let debug_agent = Agent::new();

    let session = debug_agent
        .config
        .session_manager
        .create_session(
            std::env::current_dir()?,
            "CLI Session".to_string(),
            SessionType::Hidden,
        )
        .await?;

    debug_agent.update_provider(provider, &session.id).await?;

    // Add the developer extension if available to help with debugging
    let extensions = get_all_extensions();
    for ext_wrapper in extensions {
        if ext_wrapper.enabled && ext_wrapper.config.name() == "developer" {
            if let Err(e) = debug_agent.add_extension(ext_wrapper.config).await {
                // If we can't add developer extension, continue without it
                eprintln!(
                    "Note: Could not load developer extension for debugging: {}",
                    e
                );
            }
            break;
        }
    }

    let mut debug_session = CliSession::new(
        debug_agent,
        session.id,
        false,
        None,
        None,
        None,
        None,
        "text".to_string(),
    )
    .await;

    // Process the debugging request
    println!("{}", style("Analyzing the extension failure...").yellow());
    match debug_session.headless(debug_prompt).await {
        Ok(_) => {
            println!(
                "{}",
                style("Debugging chat completed. Check the suggestions above.").green()
            );
        }
        Err(e) => {
            eprintln!("{}", style(format!("Debugging chat failed: {}", e)).red());
        }
    }
    Ok(())
}

async fn load_extensions(
    agent: Agent,
    extensions_to_load: Vec<(String, ExtensionConfig)>,
    provider_for_debug: Arc<dyn biorouter::providers::base::Provider>,
    interactive: bool,
) -> Arc<Agent> {
    let mut set = JoinSet::new();
    let agent_ptr = Arc::new(agent);

    let mut waiting_ids: BTreeSet<usize> = (0..extensions_to_load.len()).collect();
    for (id, (_label, extension)) in extensions_to_load.iter().enumerate() {
        let agent_ptr = agent_ptr.clone();
        let cfg = extension.clone();
        set.spawn(async move { (id, agent_ptr.add_extension(cfg).await) });
    }

    let get_message = |waiting_ids: &BTreeSet<usize>| {
        let labels: Vec<String> = waiting_ids
            .iter()
            .map(|id| {
                extensions_to_load
                    .get(*id)
                    .map(|e| e.0.clone())
                    .unwrap_or_default()
            })
            .collect();
        format!(
            "starting {} extensions: {}",
            waiting_ids.len(),
            labels.join(", ")
        )
    };

    let spinner = cliclack::spinner();
    spinner.start(get_message(&waiting_ids));

    let mut offer_debug: Vec<(usize, anyhow::Error)> = Vec::new();
    while let Some(result) = set.join_next().await {
        match result {
            Ok((id, Ok(_))) => {
                waiting_ids.remove(&id);
                spinner.set_message(get_message(&waiting_ids));
            }
            Ok((id, Err(e))) => offer_debug.push((id, e.into())),
            Err(e) => tracing::error!("failed to add extension: {}", e),
        }
    }

    spinner.clear();

    for (id, err) in offer_debug {
        let label = extensions_to_load
            .get(id)
            .map(|e| e.0.clone())
            .unwrap_or_default();
        eprintln!(
            "{}",
            style(format!(
                "Warning: Failed to start extension '{}' ({}), continuing without it",
                label, err
            ))
            .yellow()
        );

        if let Err(debug_err) = offer_extension_debugging_help(
            &label,
            &err.to_string(),
            Arc::clone(&provider_for_debug),
            interactive,
        )
        .await
        {
            eprintln!("Note: Could not start debugging chat: {}", debug_err);
        }
    }

    agent_ptr
}

fn check_missing_extensions_or_exit(saved_extensions: &[ExtensionConfig], interactive: bool) {
    let missing: Vec<_> = saved_extensions
        .iter()
        .filter(|ext| get_extension_by_name(&ext.name()).is_none())
        .cloned()
        .collect();

    if !missing.is_empty() {
        let names = missing
            .iter()
            .map(|e| e.name())
            .collect::<Vec<_>>()
            .join(", ");

        if interactive {
            if !cliclack::confirm(format!(
                "Extension(s) {} from the previous chat are no longer available. Restore for this chat?",
                names
            ))
            .initial_value(true)
            .interact()
            .unwrap_or(false)
            {
                println!("{}", style("Resume cancelled.").yellow());
                process::exit(0);
            }
        } else {
            eprintln!(
                "{}",
                style(format!(
                    "Warning: Extension(s) {} from the previous chat are no longer available, continuing without them.",
                    names
                ))
                .yellow()
            );
        }
    }
}

/// #31: the private, per-run session store backing a `--no-session` run.
///
/// `--no-session` is documented as "run without storing a session file", yet
/// it used to create a hidden session in the SHARED
/// `<data_dir>/sessions/sessions.db` — every message of every headless run
/// landed in (and contended on) the same store the desktop app and daemon
/// write. This builds a `SessionManager` rooted in a fresh temp directory
/// instead, giving each `--no-session` run structural isolation. The
/// returned `TempDir` deletes the store when dropped. `build_session`'s
/// early-exit paths call [`close_ephemeral_store_with_manager`] before
/// `process::exit` (which skips destructors) so the pool is closed and the
/// directory removed; panics unwind and drop it normally.
fn ephemeral_session_store() -> anyhow::Result<(tempfile::TempDir, Arc<SessionManager>)> {
    let dir = tempfile::Builder::new()
        .prefix("biorouter-no-session-")
        .tempdir()?;
    let manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
    Ok((dir, manager))
}

/// #31: `process::exit` skips destructors, so bailing out of `build_session`
/// after the private `--no-session` store was created would leak a
/// `biorouter-no-session-*` directory under the OS temp root on every early
/// exit. Close the store explicitly — **pool first, then the directory** —
/// and let the caller `process::exit`.
///
/// The ordering matters: by the later early-exit paths the agent's SQLite
/// pool is already open on this directory (WAL + -shm files). Deleting the
/// directory while those handles are open happens to work on Unix, but on
/// platforms where unlinking open files fails (Windows) the removal errors
/// and the directory leaks. Closing the pool first releases the handles, so
/// the deletion is reliable everywhere.
async fn close_ephemeral_store_with_manager(
    session_manager: &SessionManager,
    ephemeral_store_dir: Option<tempfile::TempDir>,
) {
    if ephemeral_store_dir.is_some() {
        session_manager.close().await;
    }
    // ⚠ **On a blocking thread, because the retry SLEEPS and this is `async`.**
    //
    // `close_ephemeral_store` waits the OS out with `std::thread::sleep`. Called
    // straight from here it parks whatever thread is driving this future — and
    // under `#[tokio::test]`, which is a CURRENT-THREAD runtime, that is the
    // only one there is. Nothing tokio-driven can progress while we wait for a
    // handle whose release may need exactly that, so the retry re-observes the
    // same locked file every time and a longer budget only sleeps longer.
    //
    // Measured on windows-latest, and it is why the previous attempt at this bug
    // failed: raising the budget from 2 s to ~17.8 s did not help, and the
    // diagnostic showed a removal attempted AFTER the whole budget still failing
    // with os error 32. A handle held for 17.8 seconds is not a slow handle.
    //
    // `spawn_blocking` puts the sleeping on tokio's blocking pool, which exists
    // for precisely this, and leaves the runtime free. The generous budget stays
    // — the two fixes address different halves, and a genuinely slow release on
    // a loaded runner still needs waiting out.
    //
    close_ephemeral_store(ephemeral_store_dir).await;
}

/// The budget for re-trying a store removal the OS is still refusing: how
/// many attempts, and how long to pause between them.
///
/// The pause *backs off* instead of staying flat, because the two cases it
/// serves pull in opposite directions. The common one is a handle a
/// millisecond from release, which wants the first retry almost immediately.
/// The rare one is a runner under enough load that the same handle takes
/// seconds, which wants a budget long enough to outlast it. A flat 50 ms over
/// 40 attempts served the first well and the second not at all: 2 s total,
/// and it went red on windows-latest while two sibling tests doing the
/// identical thing passed in the same run — which is what a budget slightly
/// too short looks like — though the diagnosis was WRONG, and the correction is
/// worth reading before touching these numbers again.
///
/// ⚠ **The budget was never the bug, and a big one costs more than it looks.**
/// Raising it to ~17.8 s did not fix Windows: the diagnostic showed a removal
/// attempted after the whole budget still failing with os error 32. The real
/// defect was that the wait ran ON the runtime thread — see
/// `close_ephemeral_store_with_manager`, which now uses `spawn_blocking`.
///
/// Meanwhile the long budget did real damage. Many tests in this binary take
/// ONE process-global `env_lock`, and a test that spends the whole budget holds
/// it the entire time, so every other test queues behind it. A local run went
/// from about forty seconds to over an hour with all sixteen threads parked,
/// and `privacy_tier` alone measured 17.49 s — one budget, exactly.
///
/// So: modest again. Doubling from 25 ms to a 200 ms ceiling spends ~3.6 s over
/// 20 attempts, still reaches the first retry twice as fast as the original flat
/// pause, and no longer holds a global lock for the better part of a minute.
const STORE_REMOVAL_ATTEMPTS: u32 = 20;
const STORE_REMOVAL_FIRST_PAUSE: std::time::Duration = std::time::Duration::from_millis(25);
const STORE_REMOVAL_MAX_PAUSE: std::time::Duration = std::time::Duration::from_millis(200);

/// The pause before retry number `attempt` (1-based): double the last one,
/// stopping at [`STORE_REMOVAL_MAX_PAUSE`].
fn store_removal_pause(attempt: u32) -> std::time::Duration {
    let doublings = attempt.saturating_sub(1).min(16);
    (STORE_REMOVAL_FIRST_PAUSE * (1u32 << doublings)).min(STORE_REMOVAL_MAX_PAUSE)
}

/// Remove a directory, re-trying while the OS says something still has it open.
///
/// ⚠ Closing the SQLite pool is NOT enough on Windows, which is the whole
/// reason this exists. Measured on windows-latest: with
/// `SessionManager::close()` awaited AND a following query already refused
/// because the pool is closed, removing the store still fails with os error
/// 32, "The process cannot access the file because it is being used by another
/// process", and an immediate second attempt fails identically. sqlx runs each
/// SQLite connection on its own background thread, and `Pool::close()` waits
/// for the pool's bookkeeping rather than for that thread to reach
/// `sqlite3_close`, so the db, `-wal` and `-shm` handles outlive the await by a
/// little. Unix never notices, because unlinking an open file is allowed there.
///
/// Waiting is therefore the fix and not a workaround: the handles are on their
/// way out and nothing else can be asked. The pause is bounded, and entered
/// only after an attempt has already failed, so the ordinary path pays nothing.
///
/// `remove` is injected so the retry can be tested on any platform. A test that
/// could only fail on Windows would not be a test.
fn remove_dir_all_retrying<F>(path: &Path, remove: F) -> std::io::Result<()>
where
    F: FnMut(&Path) -> std::io::Result<()>,
{
    remove_dir_all_retrying_with(path, remove, std::thread::sleep)
}

/// The retry loop with its wait injected as well as its removal.
///
/// Injecting the wait is what lets a test assert the *schedule* — that the
/// pause grows, and that the budget is still bounded — without serving it.
/// Asserting the schedule against the real clock would mean a unit test that
/// sleeps for the whole 17.8 s budget to prove the budget is 17.8 s, so the
/// tail nobody can afford to wait for is exactly the part that would go
/// unverified.
fn remove_dir_all_retrying_with<F, S>(
    path: &Path,
    mut remove: F,
    mut sleep: S,
) -> std::io::Result<()>
where
    F: FnMut(&Path) -> std::io::Result<()>,
    S: FnMut(std::time::Duration),
{
    let mut last = remove(path);
    for attempt in 1..STORE_REMOVAL_ATTEMPTS {
        match last {
            Ok(()) => return Ok(()),
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => {}
        }
        sleep(store_removal_pause(attempt));
        last = remove(path);
    }
    last
}

/// Removal of the private `--no-session` store directory. Split out so the
/// cleanup itself is unit-testable (`process::exit` is not). Prefer
/// [`close_ephemeral_store_with_manager`], which also closes the SQLite pool
/// first.
///
/// The `TempDir` is consumed either way: `close()` forgets itself whether or
/// not it succeeded, so the retry below owns the path alone and cannot race a
/// second removal from `Drop`.
pub(super) async fn close_ephemeral_store(ephemeral_store_dir: Option<tempfile::TempDir>) {
    // The retry sleeps, and Windows may need the async runtime to keep moving
    // while sqlx's connection threads release their final file handles.
    let _ =
        tokio::task::spawn_blocking(move || close_ephemeral_store_blocking(ephemeral_store_dir))
            .await;
}

fn close_ephemeral_store_blocking(ephemeral_store_dir: Option<tempfile::TempDir>) {
    if let Some(dir) = ephemeral_store_dir {
        let path = dir.path().to_path_buf();
        if dir.close().is_ok() {
            return;
        }
        if let Err(e) = remove_dir_all_retrying(&path, |p| std::fs::remove_dir_all(p)) {
            tracing::warn!(
                "failed to remove the --no-session session store on exit: {}",
                e
            );
        }
    }
}

/// The message for a run that cannot name a provider or a model, or `None`
/// when both are resolved.
///
/// Takes the already-resolved values rather than reading config itself, so the
/// two callers keep their own precedence and this stays a pure function a test
/// can drive directly. `build_session` resolves four slots (CLI flag, the
/// provider saved on the session row, a workflow pin, the global default); the
/// pre-session guard `cli::refuse_unconfigured_before_creating_a_row` resolves
/// the same list minus the saved one, which is empty by construction at the
/// point it asks, because it is about to create that row.
///
/// ⚠ This doc comment described that guard correctly while the guard did not:
/// the *callers* passed only the flags, so the global default never reached
/// here and a configured install was refused (v1.89.0 … v1.90.2). The
/// resolution now happens inside the guard, where a call site cannot omit it —
/// `cli::resolution_for_a_new_row`.
///
/// The provider is reported first when both are missing: `biorouter configure`
/// walks the user through provider and model together, so naming two problems
/// would describe one fix twice.
///
/// The flag is named against the commands that actually accept it. `biorouter`
/// with no subcommand takes no `--provider`, so "pass --provider for this run"
/// was an instruction that could not be followed on the one path a new user is
/// most likely to be reading it from.
pub fn unconfigured_precondition(provider: Option<&str>, model: Option<&str>) -> Option<String> {
    if provider.is_none() {
        return Some(
            "No provider is configured.\n\
             Run `biorouter configure` to set one up, or pass --provider <name> to \
             `biorouter session` or `biorouter run`."
                .to_string(),
        );
    }
    if model.is_none() {
        return Some(
            "No model is configured.\n\
             Run `biorouter configure` to set one up, or pass --model <name> to \
             `biorouter session` or `biorouter run`."
                .to_string(),
        );
    }
    None
}

/// An [`Agent`] whose session manager is the given (private) store, with every
/// other config knob identical to [`Agent::new`].
fn agent_with_session_manager(session_manager: Arc<SessionManager>) -> Agent {
    Agent::with_config(AgentConfig::new(
        session_manager,
        PermissionManager::instance(),
        None,
        Config::global()
            .get_biorouter_mode()
            .unwrap_or(BioRouterMode::Auto),
    ))
}

#[allow(clippy::too_many_lines)]
pub async fn build_session(session_config: SessionBuilderConfig) -> CliSession {
    let config = Config::global();
    // #31: `--no-session` gets a private per-run store so it can never write
    // to — or contend on — the shared sessions.db.
    let mut ephemeral_store_dir: Option<tempfile::TempDir> = None;
    let agent: Agent = if session_config.no_session {
        match ephemeral_session_store() {
            Ok((dir, manager)) => {
                let agent = agent_with_session_manager(manager);
                ephemeral_store_dir = Some(dir);
                agent
            }
            Err(e) => {
                output::render_error(&format!(
                    "Failed to create the private session store for --no-session: {}",
                    e
                ));
                process::exit(1);
            }
        }
    } else {
        Agent::new()
    };
    let session_manager = agent.config.session_manager.clone();

    let (saved_provider, saved_model_config) = if session_config.resume {
        if let Some(ref session_id) = session_config.session_id {
            match session_manager.get_session(session_id, false).await {
                Ok(session_data) => (session_data.provider_name, session_data.model_config),
                Err(_) => (None, None),
            }
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    let workflow = session_config.workflow.as_ref();
    let workflow_settings = workflow.and_then(|r| r.settings.as_ref());

    // Issue #56 Task 31. The same four-slot precedence as before, with a record
    // of WHICH slot won. Three of the four are things the user did not type just
    // now — a provider stored on the row months ago, a pin inside a workflow
    // somebody mailed them, the global default — so "why will this chat not
    // start" has four answers and only one of them is obvious.
    let workflow_provider = workflow_settings.and_then(|s| s.biorouter_provider.clone());
    let (resolved_provider, provider_source) =
        match (session_config.provider, saved_provider, workflow_provider) {
            (Some(p), _, _) => (Some(p), ProviderSource::CliFlag),
            (None, Some(p), _) => (Some(p), ProviderSource::SavedSession),
            (None, None, Some(p)) => (Some(p), ProviderSource::Workflow),
            (None, None, None) => (
                config.get_biorouter_provider().ok(),
                ProviderSource::GlobalDefault,
            ),
        };
    let resolved_model = session_config
        .model
        .or_else(|| saved_model_config.as_ref().map(|mc| mc.model_name.clone()))
        .or_else(|| workflow_settings.and_then(|s| s.biorouter_model.clone()))
        .or_else(|| config.get_biorouter_model().ok());

    // A fresh install has neither, and this is the first thing it reaches. Both
    // slots used to be `.expect()`, so the answer to "you have not configured a
    // provider yet" was a panic: `thread 'main' panicked at`, a note about
    // RUST_BACKTRACE, and exit 101. Every other way this function can fail
    // renders a message and exits 1, and so does this one now.
    if let Some(text) =
        unconfigured_precondition(resolved_provider.as_deref(), resolved_model.as_deref())
    {
        output::render_error(&text);
        close_ephemeral_store_with_manager(&session_manager, ephemeral_store_dir).await;
        process::exit(1);
    }
    let provider_name = resolved_provider.expect("checked by unconfigured_precondition above");
    let model_name = resolved_model.expect("checked by unconfigured_precondition above");

    let model_config = if session_config.resume
        && saved_model_config
            .as_ref()
            .is_some_and(|mc| mc.model_name == model_name)
    {
        let mut config = saved_model_config.unwrap();
        if let Some(temp) = workflow_settings.and_then(|s| s.temperature) {
            config = config.with_temperature(Some(temp));
        }
        config
    } else {
        let temperature = workflow_settings.and_then(|s| s.temperature);
        match biorouter::model::ModelConfig::new(&model_name) {
            Ok(config) => config.with_temperature(temperature),
            Err(e) => {
                output::render_error(&format!("Failed to create model configuration: {}", e));
                close_ephemeral_store_with_manager(&session_manager, ephemeral_store_dir).await;
                process::exit(1);
            }
        }
    };

    // Issue #56 Task 31 / R10. The classification of the chat this provider is
    // about to be bound to. Read from the row rather than carried, because on a
    // `--resume` the row is the only thing that knows.
    //
    // A read failure reads Public, and that is not a fail-open: this refusal is
    // an early, well-worded explanation, never the authority. Gate A — the
    // `WHERE` clause of `bind_provider_if_allowed`, a few lines below — is the
    // authority, and it fails closed on its own. Refusing here on an unreadable
    // row would only replace a specific message with a vaguer one.
    let session_classification = match session_config.session_id.as_deref() {
        Some(id) => session_manager
            .get_session(id, false)
            .await
            .map(|s| s.privacy_tier)
            .unwrap_or(biorouter::privacy::SessionClassification::Public),
        // `--no-session` creates its row below, in the private per-run store: a
        // brand-new chat, so Public.
        None => biorouter::privacy::SessionClassification::Public,
    };

    // ⚠ BEFORE `providers::create` and before the bind, so a refused chat never
    // builds a provider and never sends anything. `builder::tests::
    // the_privacy_check_runs_before_the_provider_is_ever_created` is what keeps
    // that ordering.
    if let Some(text) = privacy_start_refusal(
        &provider_name,
        provider_source,
        session_classification,
        session_config.session_id.as_deref(),
        workflow_settings,
    )
    .await
    {
        output::render_error(&text);
        close_ephemeral_store_with_manager(&session_manager, ephemeral_store_dir).await;
        process::exit(1);
    }

    let new_provider = match create(&provider_name, model_config).await {
        Ok(provider) => provider,
        Err(e) => {
            output::render_error(&format!(
                "Error {e}.{}",
                keyring_advice(&provider_name).await
            ));
            close_ephemeral_store_with_manager(&session_manager, ephemeral_store_dir).await;
            process::exit(1);
        }
    };
    let provider_for_display = Arc::clone(&new_provider);

    if let Some(lead_worker) = new_provider.as_lead_worker() {
        let (lead_model, worker_model) = lead_worker.get_model_info();
        tracing::info!(
            "Lead/worker mode enabled: lead model (first 3 turns): {}, worker model (turn 4+): {}, auto-fallback on failures: enabled",
            lead_model,
            worker_model
        );
    } else {
        tracing::info!("Using model: {}", model_name);
    }

    let session_id: String = if session_config.no_session {
        let working_dir = std::env::current_dir().expect("Could not get working directory");
        // The hidden session lives in the PRIVATE per-run store built above,
        // never in the shared sessions.db (#31).
        let session = match session_manager
            .create_session(working_dir, "CLI Session".to_string(), SessionType::Hidden)
            .await
        {
            Ok(session) => session,
            Err(e) => {
                let store = ephemeral_store_dir
                    .as_ref()
                    .map(|d| d.path().join("sessions").join("sessions.db"))
                    .unwrap_or_default();
                output::render_error(&format!(
                    "Could not initialize the --no-session session store at {}: {}",
                    store.display(),
                    e
                ));
                close_ephemeral_store_with_manager(&session_manager, ephemeral_store_dir).await;
                process::exit(1);
            }
        };
        session.id
    } else if session_config.resume {
        if let Some(session_id) = session_config.session_id {
            match session_manager.get_session(&session_id, false).await {
                Ok(_) => session_id,
                Err(_) => {
                    output::render_error(&format!(
                        "Cannot resume session {} - no such session exists",
                        style(&session_id).cyan()
                    ));
                    close_ephemeral_store_with_manager(&session_manager, ephemeral_store_dir).await;
                    process::exit(1);
                }
            }
        } else {
            match session_manager.list_sessions().await {
                Ok(sessions) if !sessions.is_empty() => sessions[0].id.clone(),
                _ => {
                    output::render_error("Cannot resume - no previous chats found");
                    close_ephemeral_store_with_manager(&session_manager, ephemeral_store_dir).await;
                    process::exit(1);
                }
            }
        }
    } else {
        session_config.session_id.unwrap()
    };

    if let Err(e) = agent.update_provider(new_provider, &session_id).await {
        output::render_error(&format!("Failed to initialize agent: {}", e));
        close_ephemeral_store_with_manager(&session_manager, ephemeral_store_dir).await;
        process::exit(1);
    }

    if session_config.resume {
        let session = match agent
            .config
            .session_manager
            .get_session(&session_id, false)
            .await
        {
            Ok(session) => session,
            Err(e) => {
                output::render_error(&format!("Failed to read session metadata: {}", e));
                close_ephemeral_store_with_manager(&session_manager, ephemeral_store_dir).await;
                process::exit(1);
            }
        };

        let current_workdir =
            std::env::current_dir().expect("Failed to get current working directory");
        if current_workdir != session.working_dir {
            if session_config.interactive {
                let change_workdir = cliclack::confirm(format!("{} The original working directory of this chat was set to {}. Your current directory is {}. Do you want to switch back to the original working directory?", style("WARNING:").yellow(), style(session.working_dir.display()).cyan(), style(current_workdir.display()).cyan()))
                        .initial_value(true)
                        .interact().expect("Failed to get user input");

                if change_workdir {
                    if !session.working_dir.exists() {
                        output::render_error(&format!(
                            "Cannot switch to original working directory - {} no longer exists",
                            style(session.working_dir.display()).cyan()
                        ));
                    } else if let Err(e) = std::env::set_current_dir(&session.working_dir) {
                        output::render_error(&format!(
                            "Failed to switch to original working directory: {}",
                            e
                        ));
                    }
                }
            } else {
                eprintln!(
                    "{}",
                    style(format!(
                        "Warning: Working directory differs from the chat (current: {}, chat: {}). Staying in current directory.",
                        current_workdir.display(),
                        session.working_dir.display()
                    ))
                    .yellow()
                );
            }
        }
    }

    // Setup extensions for the agent
    // Extensions need to be added after the session is created because we change directory when resuming a session

    for warning in biorouter::config::get_warnings() {
        eprintln!("{}", style(format!("Warning: {}", warning)).yellow());
    }

    let configured_extensions: Vec<ExtensionConfig> = if session_config.resume {
        agent
            .config
            .session_manager
            .get_session(&session_id, false)
            .await
            .ok()
            .and_then(|s| EnabledExtensionsState::from_extension_data(&s.extension_data))
            .map(|state| {
                check_missing_extensions_or_exit(&state.extensions, session_config.interactive);
                state.extensions
            })
            .unwrap_or_else(get_enabled_extensions)
    } else {
        let mut resolved = resolve_extensions_for_new_session(
            workflow.and_then(|r| r.extensions.as_deref()),
            None,
        );
        // A workflow that declares a knowledge selection needs the Knowledge
        // capability to honour it. The scheduler and the server both do this;
        // the CLI did not, so `biorouter run --workflow` could load a workflow
        // whose `knowledge_bases:` key had nowhere to land.
        if let Some(workflow) = workflow {
            biorouter::workflow::runtime::ensure_required_extensions(workflow, &mut resolved);
        }
        resolved
    };

    let cli_flag_extensions_to_load = parse_cli_flag_extensions(
        &session_config.extensions,
        &session_config.streamable_http_extensions,
        &session_config.builtins,
    );

    let mut extensions_to_load: Vec<(String, ExtensionConfig)> = configured_extensions
        .iter()
        .map(|cfg| (cfg.name(), cfg.clone()))
        .collect();
    extensions_to_load.extend(cli_flag_extensions_to_load);

    let agent_ptr = load_extensions(
        agent,
        extensions_to_load,
        Arc::clone(&provider_for_display),
        session_config.interactive,
    )
    .await;

    // Determine editor mode
    let edit_mode = config
        .get_param::<String>("EDIT_MODE")
        .ok()
        .and_then(|edit_mode| match edit_mode.to_lowercase().as_str() {
            "emacs" => Some(EditMode::Emacs),
            "vi" => Some(EditMode::Vi),
            _ => {
                eprintln!("Invalid EDIT_MODE specified, defaulting to Emacs");
                None
            }
        });

    let debug_mode = session_config.debug || config.get_param("BIOROUTER_DEBUG").unwrap_or(false);

    let mut session = CliSession::new(
        Arc::try_unwrap(agent_ptr).unwrap_or_else(|_| panic!("There should be no more references")),
        session_id.clone(),
        debug_mode,
        session_config.scheduled_job_id.clone(),
        session_config.max_turns,
        edit_mode,
        workflow.and_then(|r| r.retry.clone()),
        session_config.output_format.clone(),
    )
    .await;
    // #31: keep the private --no-session store alive for the whole run; its
    // temp dir is deleted (best-effort) when the session is dropped.
    if let Some(dir) = ephemeral_store_dir {
        session.hold_ephemeral_store_dir(dir);
    }

    if let Err(e) = session
        .agent
        .persist_extension_state(&session_id.clone())
        .await
    {
        tracing::warn!("Failed to save extension state: {}", e);
    }

    // Add CLI-specific system prompt extension
    session
        .agent
        .extend_system_prompt(super::prompt::get_cli_prompt())
        .await;

    if let Some(additional_prompt) = session_config.additional_system_prompt {
        session.agent.extend_system_prompt(additional_prompt).await;
    }

    // Install the workflow through the SAME path the desktop and the scheduler
    // use. This used to be a bare `apply_workflow_components` far above (sub
    // workflows and response schema only) plus `workflow.instructions` shoved
    // in as a raw `additional_system_prompt`, so `biorouter run --workflow`
    // silently dropped `skills:` and `knowledge_bases:` while every other
    // surface honoured them.
    //
    // It runs here, not earlier, because `prepare_prompt` resolves declared
    // skills against the SESSION — which does not exist until now.
    if let Some(workflow) = workflow {
        let prepared = match biorouter::workflow::runtime::prepare_prompt(
            session.agent.config.session_manager.as_ref(),
            &session_id,
            workflow,
        )
        .await
        {
            Ok(prompt) => prompt,
            Err(err) => {
                // A workflow naming a skill that is not installed is a broken
                // run, not a run with a missing hint: the instructions the
                // skill carries are load-bearing. Fail with the name.
                output::render_error(&format!("Failed to prepare workflow: {err}"));
                process::exit(1);
            }
        };
        if let Err(err) = biorouter::workflow::runtime::install_prepared(
            &session.agent,
            &session_id,
            workflow,
            true,
            prepared,
        )
        .await
        {
            output::render_error(&format!("Failed to apply workflow: {err}"));
            process::exit(1);
        }
    }

    // Only override system prompt if a system override exists
    let system_prompt_file: Option<String> =
        config.get_param("BIOROUTER_SYSTEM_PROMPT_FILE_PATH").ok();
    if let Some(ref path) = system_prompt_file {
        let override_prompt =
            std::fs::read_to_string(path).expect("Failed to read system prompt file");
        session.agent.override_system_prompt(override_prompt).await;
    }

    // Display session information unless in quiet mode
    if !session_config.quiet {
        // Issue #56 Task 31 / R10. Re-read rather than reusing the value the
        // start-time check sampled: the bind above ratchets nothing, but a
        // `--no-session` run had no row to read then and has one now, and a
        // resumed chat can have been raised by another process in between.
        let privacy = session_manager
            .get_session(&session_id, false)
            .await
            .map(|s| s.privacy_tier)
            .unwrap_or(session_classification);
        output::display_session_info(
            session_config.resume,
            &provider_name,
            &model_name,
            &Some(session_id),
            Some(&provider_for_display),
            privacy,
        );
    }
    session
}

/// The keychain paragraph, or nothing.
///
/// Provider construction fails for a credential reason often enough that the
/// advice earned its place — but it is advice about SECRETS, and not every
/// provider has any. `claude_code` and `codex` drive a CLI the user signed in
/// to; Biorouter never sees a credential for either and stores nothing for
/// them, so telling a user whose `claude` binary is simply not on PATH to
/// "check your system keychain and run 'biorouter configure' again" sends them
/// to a place with nothing in it, and buries the error that named the real
/// problem underneath three lines of it.
///
/// The question is asked of the provider's own declared config keys rather than
/// of a list of names kept here, so a provider added later is classified by
/// what it declares. A provider the registry cannot describe — including one it
/// has never heard of, which is one of the ways `create` fails — keeps the
/// advice: it may well have secrets, and an unnecessary paragraph is a much
/// smaller failure than withholding the one that would have helped.
async fn keyring_advice(provider_name: &str) -> &'static str {
    const ADVICE: &str = "\n\
        Please check your system keychain and run 'biorouter configure' again.\n\
        If your system is unable to use the keyring, please try setting secret key(s) via environment variables.\n\
        For more info, see: https://BaranziniLab.github.io/biorouter/docs/troubleshooting/#keychainkeyring-errors";
    let has_secrets = biorouter::providers::providers()
        .await
        .into_iter()
        .find(|(metadata, _)| metadata.name == provider_name)
        .map(|(metadata, _)| metadata.config_keys.iter().any(|key| key.secret));
    match has_secrets {
        Some(false) => "",
        _ => ADVICE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two coding-agent providers declare no config keys at all: they drive
    /// a CLI the user signed in to, and Biorouter holds no credential for
    /// either. A `could not find the 'claude' command` failure followed by
    /// three lines about the system keychain sends the reader somewhere with
    /// nothing in it.
    #[tokio::test]
    async fn a_provider_with_no_secrets_is_not_told_to_check_its_keychain() {
        for provider in ["claude_code", "codex"] {
            assert_eq!(
                keyring_advice(provider).await,
                "",
                "{provider} stores no secret"
            );
        }
    }

    /// …and the advice is kept for everything that does hold one, including a
    /// provider the registry cannot describe: an unnecessary paragraph is a far
    /// smaller failure than withholding the one that would have helped.
    #[tokio::test]
    async fn a_provider_with_secrets_still_gets_the_keychain_advice() {
        for provider in ["anthropic", "openai", "no_such_provider_exists"] {
            assert!(
                keyring_advice(provider).await.contains("system keychain"),
                "{provider} must keep the advice"
            );
        }
    }

    /// #31: the `--no-session` store must be a private per-run directory —
    /// never the shared `<data_dir>/sessions/sessions.db` — and it must be a
    /// fully functional session store (create / write / read back).
    #[tokio::test]
    async fn no_session_store_is_private_and_functional() {
        let shared_root = biorouter::config::paths::Paths::data_dir();
        let shared_db = shared_root.join("sessions").join("sessions.db");
        let shared_mtime_before = std::fs::metadata(&shared_db)
            .and_then(|m| m.modified())
            .ok();

        let (dir, manager) = ephemeral_session_store().expect("ephemeral store");
        assert_ne!(dir.path(), shared_root.as_path());
        assert!(
            !dir.path().starts_with(&shared_root),
            "the ephemeral store must not live under the shared data dir ({})",
            shared_root.display()
        );

        // The exact operations a --no-session run performs against its store.
        let session = manager
            .create_session(
                dir.path().to_path_buf(),
                "CLI Session".to_string(),
                SessionType::Hidden,
            )
            .await
            .expect("create session in the private store");
        manager
            .add_message(
                &session.id,
                &biorouter::conversation::message::Message::user().with_text("hello"),
            )
            .await
            .expect("write a message to the private store");
        let loaded = manager
            .get_session(&session.id, true)
            .await
            .expect("read back from the private store");
        assert_eq!(loaded.conversation.unwrap().len(), 1);

        // The writes landed in the private store...
        assert!(
            dir.path().join("sessions").join("sessions.db").exists(),
            "the private sessions.db must exist under the temp dir"
        );
        // ...and the shared sessions.db was untouched (same mtime, or still
        // absent) throughout the run.
        let shared_mtime_after = std::fs::metadata(&shared_db)
            .and_then(|m| m.modified())
            .ok();
        assert_eq!(
            shared_mtime_before, shared_mtime_after,
            "a --no-session run must not touch the shared session store"
        );
    }

    /// A run that cannot name a provider or a model is refused with something
    /// the user can act on.
    ///
    /// Both slots used to be `.expect()`, so the first thing a fresh install
    /// saw was `thread 'main' panicked at`, a backtrace note and exit 101. The
    /// text below is the contract that replaced it, and each assertion rules
    /// out a specific wrong version: dropping the model check (it is the
    /// second slot and the easy one to forget), reporting the model when the
    /// provider is the thing that is missing, and a message that states the
    /// problem without naming the fix.
    #[test]
    fn an_unconfigured_run_is_told_what_to_run() {
        assert_eq!(
            unconfigured_precondition(Some("anthropic"), Some("opus")),
            None
        );

        let no_provider = unconfigured_precondition(None, Some("opus"))
            .expect("a run with no provider must be refused");
        assert!(no_provider.contains("provider"));
        assert!(
            no_provider.contains("biorouter configure"),
            "the refusal must name the command that fixes it: {no_provider}"
        );

        let no_model = unconfigured_precondition(Some("anthropic"), None)
            .expect("a run with no model must be refused, not just one with no provider");
        assert!(no_model.contains("model"));
        assert!(no_model.contains("biorouter configure"));

        let neither =
            unconfigured_precondition(None, None).expect("a run with neither must be refused");
        assert_eq!(
            neither, no_provider,
            "with both missing, report the provider: `biorouter configure` sets up both, \
             so naming two problems describes one fix twice"
        );
    }

    /// #31: the explicit early-exit cleanup removes the private store (and
    /// tolerates `None`). `process::exit` itself is untestable; this pins the
    /// removal half that every early-exit path runs.
    ///
    /// The pool is closed first because production always has: the only caller
    /// of this bare helper is [`close_ephemeral_store_with_manager`], which
    /// closes the pool before delegating here. Dropping the
    /// `Arc<SessionManager>` is NOT a substitute — an sqlx pool does not close
    /// synchronously on drop, so the db + WAL/-shm handles stay open and
    /// `remove_dir_all` fails on platforms that refuse to unlink open files
    /// (Windows), where this test used to leak the directory and fail. The
    /// pool-closed assertion keeps that precondition load-bearing: delete the
    /// `close().await` below and this test fails on every platform, not just
    /// the one that can observe the handles.
    #[tokio::test]
    async fn close_ephemeral_store_removes_the_directory() {
        let (dir, manager) = ephemeral_session_store().expect("ephemeral store");
        let path = dir.path().to_path_buf();
        manager
            .create_session(path.clone(), "CLI Session".to_string(), SessionType::Hidden)
            .await
            .expect("create session");
        assert!(path.exists());

        manager.close().await;
        assert!(
            manager
                .create_session(path.clone(), "again".to_string(), SessionType::Hidden)
                .await
                .is_err(),
            "the pool must be closed before the directory is removed, or the store's \
             open handles make the removal fail where open files cannot be unlinked"
        );

        close_ephemeral_store(Some(dir)).await;
        assert!(
            !path.exists(),
            "an early exit must not leak the biorouter-no-session-* directory"
        );
        // A run without --no-session has no store to close.
        close_ephemeral_store(None).await;
    }

    /// #31 ordering: the early-exit cleanup closes the SQLite pool BEFORE
    /// deleting the temp directory, releasing the WAL/-shm handles that made
    /// the removal fail (and leak the dir) on platforms where deleting open
    /// files is not allowed. On Unix the deletion succeeds either way, so
    /// the pool-closed assertion is the cross-platform proof of ordering.
    #[tokio::test]
    async fn early_exit_cleanup_closes_the_pool_before_deleting_the_store() {
        let (dir, manager) = ephemeral_session_store().expect("ephemeral store");
        let path = dir.path().to_path_buf();
        // Open the pool with real traffic so WAL files exist.
        manager
            .create_session(path.clone(), "CLI Session".to_string(), SessionType::Hidden)
            .await
            .expect("create session");

        close_ephemeral_store_with_manager(&manager, Some(dir)).await;

        assert!(
            !path.exists(),
            "the biorouter-no-session-* directory must be removed{}",
            why_the_store_survived(&path)
        );
        assert!(
            manager
                .create_session(path, "again".to_string(), SessionType::Hidden)
                .await
                .is_err(),
            "the pool must be closed, not still writing into deleted files"
        );

        // A run without --no-session must leave its (shared) store untouched.
        let (dir2, manager2) = ephemeral_session_store().expect("ephemeral store");
        close_ephemeral_store_with_manager(&manager2, None).await;
        manager2
            .create_session(
                dir2.path().to_path_buf(),
                "still open".to_string(),
                SessionType::Hidden,
            )
            .await
            .expect("no dir handed over means the store must stay usable");
    }

    /// #31 ordering, the half no Unix runner can observe. The behavioural test
    /// above proves the pool ENDS closed; it cannot prove the pool closed
    /// FIRST, because on Unix the directory is removed successfully either way.
    /// Swapping the two statements in
    /// [`close_ephemeral_store_with_manager`] keeps every assertion up there
    /// green on macOS and Linux while silently restoring the Windows leak the
    /// ordering exists to prevent — measured, not assumed.
    ///
    /// So this is a source-order tripwire, in the style of
    /// `the_privacy_check_runs_before_the_provider_is_ever_created` below: it
    /// catches the realistic regression — someone reorders or "tidies" the two
    /// statements — and nothing subtler.
    #[test]
    fn the_pool_is_closed_before_the_store_directory_is_removed() {
        let src = include_str!("builder.rs");
        let (_, after_signature) = src
            .split_once("async fn close_ephemeral_store_with_manager(")
            .expect("the early-exit cleanup helper is gone");
        // Just this function's body: the first unindented `}` closes it.
        let (body, _) = after_signature
            .split_once("\n}\n")
            .expect("could not find the end of close_ephemeral_store_with_manager");

        // ⚠ **Comment lines stripped before ANY assertion below.**
        //
        // Every needle this test greps for is also NAMED in the prose that
        // explains it — that is what good comments in this file look like. So a
        // grep over the raw body is satisfied by the explanation of a line that
        // is no longer there, and the test passes on its own documentation.
        //
        // Measured, not theorised: the `spawn_blocking` assertion below was
        // added with a comment beside it that used the word, and deleting the
        // real call left the test GREEN. A tripwire that cannot fail is worse
        // than no tripwire, because it is counted as coverage.
        let body: String = body
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let body = body.as_str();

        let close_pool = body.find("session_manager.close().await").expect(
            "close_ephemeral_store_with_manager no longer closes the SQLite pool, so every \
             early exit leaves the store's db + WAL/-shm handles open",
        );
        let remove_dir = body
            .find("close_ephemeral_store(ephemeral_store_dir)")
            .expect("close_ephemeral_store_with_manager no longer removes the store directory");
        assert!(
            close_pool < remove_dir,
            "the pool close moved BELOW the directory removal; the store's handles are still \
             open when the directory is deleted, which fails — and leaks the \
             biorouter-no-session-* directory — on platforms that refuse to unlink open files"
        );

        // ⚠ And the removal must not run ON the runtime thread.
        //
        // `close_ephemeral_store` waits the OS out with `std::thread::sleep`.
        // Called directly from this `async fn` it parks whatever thread drives
        // the future, and under `#[tokio::test]` — a CURRENT-THREAD runtime —
        // that is the only one. Nothing tokio-driven can progress while we wait
        // for a handle whose release may need exactly that.
        //
        // This is not theoretical and it is not a style rule. It shipped:
        // `early_exit_cleanup_closes_the_pool_before_deleting_the_store` went
        // red on windows-latest, a first fix raised the retry budget from 2 s to
        // ~17.8 s on the theory that the wait was too short, and it went red
        // again — with `why_the_store_survived` reporting a removal attempted
        // AFTER the entire budget still failing with os error 32. A handle held
        // for 17.8 seconds is not a slow handle; it is one nothing was allowed
        // to release.
        //
        // A source tripwire for the same reason the ordering above is one: the
        // failure it prevents is only observable on a runner nobody can
        // reproduce locally, and the realistic regression is someone "tidying"
        // the `spawn_blocking` away because the call looks synchronous anyway.
        let (_, after_remove_signature) = src
            .split_once("pub(super) async fn close_ephemeral_store(")
            .expect("the asynchronous store-removal helper is gone");
        let (remove_body, _) = after_remove_signature
            .split_once("\n}\n")
            .expect("could not find the end of close_ephemeral_store");
        let remove_body: String = remove_body
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            remove_body.contains("spawn_blocking"),
            "the store removal is being awaited on the runtime thread again. It sleeps, so \
             on a current-thread runtime it parks the only thread there is and the retry \
             re-observes the same locked file until the budget runs out. Put it back on \
             tokio's blocking pool."
        );
    }

    /// #31: when a run ends, the private store is gone.
    ///
    /// ⚠ This deliberately does NOT assert on `TempDir`'s own `Drop`, and the
    /// reason is measured rather than assumed. `Drop` makes exactly one
    /// `remove_dir_all` attempt and discards the error, and on windows-latest
    /// that attempt loses: with the pool closed and a following query already
    /// refused, the removal still failed with os error 32, "The process cannot
    /// access the file because it is being used by another process", and so
    /// did an immediate retry. sqlx reaches `sqlite3_close` on a
    /// per-connection background thread, so the handles outlive
    /// `Pool::close().await`.
    ///
    /// A single best-effort attempt is therefore not a contract any test can
    /// hold on Windows, and production does not depend on one:
    /// `CliSession::close_ephemeral_store` is what ends a real run, and it
    /// closes the pool and then removes the directory through
    /// [`close_ephemeral_store`], which waits the handles out. That is the
    /// path exercised here.
    #[tokio::test]
    async fn a_finished_run_leaves_no_private_store_behind() {
        let (dir, manager) = ephemeral_session_store().expect("ephemeral store");
        let path = dir.path().to_path_buf();
        manager
            .create_session(path.clone(), "CLI Session".to_string(), SessionType::Hidden)
            .await
            .expect("create session");
        assert!(path.exists());

        // Exactly what `CliSession::close_ephemeral_store` does at end of run.
        close_ephemeral_store_with_manager(&manager, Some(dir)).await;

        assert!(
            !path.exists(),
            "a finished run must not leave the biorouter-no-session-* store behind{}",
            why_the_store_survived(&path)
        );
    }

    /// The retry is the fix for the Windows handle lag, so it needs a test that
    /// can fail somewhere other than Windows.
    ///
    /// Injecting the removal is what buys that: the OS behaviour being modelled
    /// is "refuses for a while, then succeeds", and a closure reproduces it on
    /// any platform. A test that could only go red on the one runner nobody can
    /// reproduce locally would leave this permanently unverified.
    #[test]
    fn a_removal_the_os_is_still_refusing_is_retried_until_it_takes() {
        use std::cell::Cell;
        use std::io::{Error, ErrorKind};

        // The wait is injected alongside the removal, so the whole budget can
        // be spent in a test that takes no time to run.
        let no_wait = |_| {};

        let calls = Cell::new(0);
        let result = remove_dir_all_retrying_with(
            Path::new("irrelevant"),
            |_| {
                calls.set(calls.get() + 1);
                if calls.get() < 3 {
                    Err(Error::other("still in use"))
                } else {
                    Ok(())
                }
            },
            no_wait,
        );
        assert!(
            result.is_ok(),
            "a removal that eventually succeeds must succeed"
        );
        assert_eq!(
            calls.get(),
            3,
            "it must keep trying past the first refusal, and stop as soon as one takes"
        );

        // It also has to give up rather than hang on a directory that is never
        // going to be removable.
        let calls = Cell::new(0);
        let result = remove_dir_all_retrying_with(
            Path::new("irrelevant"),
            |_| {
                calls.set(calls.get() + 1);
                Err(Error::new(ErrorKind::PermissionDenied, "never"))
            },
            no_wait,
        );
        assert!(
            result.is_err(),
            "a permanent failure must be reported, not swallowed"
        );
        assert_eq!(
            calls.get(),
            STORE_REMOVAL_ATTEMPTS,
            "the retry budget is bounded"
        );

        // An already-absent directory is the goal state, not a failure.
        let calls = Cell::new(0);
        let result = remove_dir_all_retrying_with(
            Path::new("irrelevant"),
            |_| {
                calls.set(calls.get() + 1);
                Err(Error::new(ErrorKind::NotFound, "gone"))
            },
            no_wait,
        );
        assert!(result.is_ok(), "already gone is success");
        assert_eq!(calls.get(), 1, "and needs no retries");
    }

    /// The budget is the fix, so the budget is what needs pinning.
    ///
    /// A flat 50 ms pause over these same 40 attempts spent 2 s in total, and
    /// 2 s was measurably not enough on a loaded windows-latest runner. Both
    /// halves of the replacement have to hold for that to be addressed: the
    /// first retry must stay prompt (a backoff that starts slow would make
    /// the ordinary case worse to fix the rare one), and the tail must be
    /// long enough that a handle taking seconds to close is still waited out.
    #[test]
    fn the_removal_budget_backs_off_and_still_outlasts_a_slow_handle() {
        use std::cell::RefCell;
        use std::io::Error;

        let waits: RefCell<Vec<std::time::Duration>> = RefCell::new(Vec::new());
        let result = remove_dir_all_retrying_with(
            Path::new("irrelevant"),
            |_| Err(Error::other("never")),
            |d| waits.borrow_mut().push(d),
        );
        assert!(result.is_err(), "a permanent failure is still reported");

        let waits = waits.into_inner();
        assert_eq!(
            waits.len() as u32,
            STORE_REMOVAL_ATTEMPTS - 1,
            "one wait between each pair of attempts, and none after the last"
        );
        assert_eq!(
            waits[0], STORE_REMOVAL_FIRST_PAUSE,
            "the first retry must stay prompt: the common case is a handle a \
             millisecond from release, and it pays this wait in full"
        );
        assert!(
            waits.windows(2).all(|w| w[1] >= w[0]),
            "the pause must never shrink: {waits:?}"
        );
        assert_eq!(
            *waits.last().expect("a bounded budget still has waits"),
            STORE_REMOVAL_MAX_PAUSE,
            "the backoff must reach its ceiling rather than growing without bound"
        );

        // ⚠ Bounded on BOTH sides, and the upper bound is the one with a story.
        //
        // A previous pass raised this budget to ~17.8 s believing the Windows
        // failure was a wait that was too short. It was not — the wait was
        // running on the runtime thread — and the oversized budget did its own
        // damage: many tests in this binary take one process-global `env_lock`,
        // so a test that spends the whole budget holds it throughout and every
        // other test queues behind it. A local run went from about forty seconds
        // to over an hour with all sixteen threads parked.
        //
        // So the ceiling is a real requirement, not tidiness. Anyone raising it
        // is re-buying that hour.
        let total: std::time::Duration = waits.iter().sum();
        assert!(
            total >= std::time::Duration::from_secs(3),
            "the budget must still outlast a handle that is merely slow to release. Got {total:?}"
        );
        assert!(
            total <= std::time::Duration::from_secs(5),
            "the budget is held under a process-global lock while it runs, so a long one \
             serialises the whole test binary behind it. If a handle needs more than this, \
             the answer is not more waiting - it is finding what is holding it. Got {total:?}"
        );
    }

    /// Diagnostics for a store directory that outlived the thing meant to
    /// remove it.
    ///
    /// `TempDir`'s `Drop` calls `remove_dir_all` and throws the error away, so
    /// a failure here says only "the path is still there" and leaves the two
    /// candidate causes indistinguishable: a file handle somebody never
    /// released, or a removal the OS refused for a reason of its own. This
    /// retries the removal in the foreground purely to capture the message,
    /// and lists what is left. It runs only on the failure path, so it cannot
    /// turn a red run green.
    fn why_the_store_survived(path: &std::path::Path) -> String {
        let entries = match std::fs::read_dir(path) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(", "),
            Err(e) => format!("<unreadable: {e}>"),
        };
        let retry = match std::fs::remove_dir_all(path) {
            Ok(()) => "a second removal SUCCEEDED, so the first lost a race".to_string(),
            Err(e) => format!("a second removal also failed: {e:?}"),
        };
        format!("\n  still present: [{entries}]\n  {retry}")
    }

    #[test]
    fn test_session_builder_config_creation() {
        let config = SessionBuilderConfig {
            session_id: None,
            resume: false,
            no_session: false,
            extensions: vec!["echo test".to_string()],
            streamable_http_extensions: vec!["http://localhost:8080/mcp".to_string()],
            builtins: vec!["developer".to_string()],
            workflow: None,
            additional_system_prompt: Some("Test prompt".to_string()),
            provider: None,
            model: None,
            debug: true,
            max_tool_repetitions: Some(5),
            max_turns: None,
            scheduled_job_id: None,
            interactive: true,
            quiet: false,
            output_format: "text".to_string(),
        };

        assert_eq!(config.extensions.len(), 1);
        assert_eq!(config.streamable_http_extensions.len(), 1);
        assert_eq!(config.builtins.len(), 1);
        assert!(config.debug);
        assert_eq!(config.max_tool_repetitions, Some(5));
        assert!(config.max_turns.is_none());
        assert!(config.scheduled_job_id.is_none());
        assert!(config.interactive);
        assert!(!config.quiet);
    }

    #[test]
    fn test_session_builder_config_default() {
        let config = SessionBuilderConfig::default();

        assert!(config.session_id.is_none());
        assert!(!config.resume);
        assert!(!config.no_session);
        assert!(config.extensions.is_empty());
        assert!(config.streamable_http_extensions.is_empty());
        assert!(config.builtins.is_empty());
        assert!(config.workflow.is_none());
        assert!(config.additional_system_prompt.is_none());
        assert!(!config.debug);
        assert!(config.max_tool_repetitions.is_none());
        assert!(config.max_turns.is_none());
        assert!(config.scheduled_job_id.is_none());
        assert!(!config.interactive);
        assert!(!config.quiet);
    }

    #[tokio::test]
    async fn test_offer_extension_debugging_help_function_exists() {
        // This test just verifies the function compiles and can be called
        // We can't easily test the interactive parts without mocking

        // We can't actually test the full function without a real provider and user interaction
        // But we can at least verify it compiles and the function signature is correct
        let extension_name = "test-extension";
        let error_message = "test error";

        // This test mainly serves as a compilation check
        assert_eq!(extension_name, "test-extension");
        assert_eq!(error_message, "test error");
    }

    /// Issue #56 Task 31, the half the plan writes as `assert_eq!(turns_started(),
    /// 0)`. There is no turn counter to read here — `build_session` calls
    /// `process::exit` and cannot be driven from a test at all — so what is
    /// pinned instead is the property that makes the count zero: the privacy
    /// check runs BEFORE `providers::create`, which is the first thing on this
    /// path that constructs anything at all, let alone sends.
    ///
    /// A source-order tripwire rather than a behavioural test, and it is written
    /// down as such: it catches the realistic regression (someone moves the
    /// check down to where the provider instance is handy, so a workflow pinning
    /// a public model gets refused only after the model has been built and the
    /// session bound) and it catches nothing subtler.
    #[test]
    fn the_privacy_check_runs_before_the_provider_is_ever_created() {
        let src = include_str!("builder.rs");
        // The CALL, not the definition: `= privacy_start_refusal(` appears only
        // at the `if let Some(text) = …` call site, so a check that stayed
        // defined but moved below `create` still fails this.
        let check = src
            .find("= privacy_start_refusal(")
            .expect("the start-time privacy check is gone from build_session");
        let create = src
            .find("create(&provider_name, model_config)")
            .expect("`providers::create` is no longer called the way this audit looks for");
        assert!(
            check < create,
            "the privacy check moved BELOW `providers::create`; a refused chat now builds a \
             provider (and, a few lines later, binds it) before saying no"
        );
    }

    #[test]
    fn test_truncate_with_ellipsis() {
        assert_eq!(truncate_with_ellipsis("abc", 5), "abc");

        assert_eq!(truncate_with_ellipsis("abcde", 5), "abcde");

        assert_eq!(truncate_with_ellipsis("abcdef", 5), "abcde…");
        assert_eq!(truncate_with_ellipsis("hello world", 5), "hello…");

        assert_eq!(truncate_with_ellipsis("", 5), "");
    }
}
