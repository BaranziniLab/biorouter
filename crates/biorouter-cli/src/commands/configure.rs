use crate::workflows::github_workflow::BIOROUTER_WORKFLOW_GITHUB_REPO_CONFIG_KEY;
use biorouter::agents::default_worker::{
    configured_default_worker_in, DefaultWorker, WORKER_MODEL_KEY, WORKER_PROVIDER_KEY,
};
use biorouter::agents::extension::ToolInfo;
use biorouter::agents::extension_manager::get_parameter_names;
use biorouter::agents::Agent;
use biorouter::agents::{extension::Envs, ExtensionConfig};
use biorouter::config::declarative_providers::{create_custom_provider, remove_custom_provider};
use biorouter::config::extensions::{
    get_all_extension_names, get_all_extensions, get_enabled_extensions, get_extension_by_name,
    name_to_key, remove_extension, set_extension, set_extension_enabled,
};
use biorouter::config::paths::Paths;
use biorouter::config::permission::PermissionLevel;
use biorouter::config::signup_tetrate::TetrateAuth;
use biorouter::config::{
    configure_tetrate, BioRouterMode, Config, ConfigError, ExperimentManager, ExtensionEntry,
    PermissionManager,
};
use biorouter::conversation::message::Message;
use biorouter::model::ModelConfig;
use biorouter::providers::provider_test::test_provider_configuration;
// Whether Ollama serves a model from ollama.com instead of this machine (a
// `cloud` or `-cloud` tag). The provider's own rule, not a copy of it: the
// copy that used to live here had to be pinned to the provider's test cases.
use biorouter::providers::decision::chat::is_decision_chat_provider;
use biorouter::providers::utils::is_ollama_cloud_model;
use biorouter::providers::{create, providers, retry_operation, RetryConfig};
use biorouter::session::SessionType;
use cliclack::spinner;
use console::style;
use serde_json::Value;
use std::collections::HashMap;
use std::io::IsTerminal;

// useful for light themes where there is no dicernible colour contrast between
// cursor-selected and cursor-unselected items.
const MULTISELECT_VISIBILITY_HINT: &str = "<";

/// What `biorouter configure` says when there is no terminal to prompt on
/// (QA-D F9). It names the non-interactive route to the same end state rather
/// than only refusing: this is the command every install guide points at, so it
/// is the one most often run from a script.
pub(crate) const CONFIGURE_NEEDS_A_TERMINAL: &str =
    "`biorouter configure` is interactive and needs a terminal; without one, choose a model \
     with `biorouter models set --provider <provider> --model <model>` (`biorouter models \
     providers` lists the providers) and pass the provider's API key in its environment \
     variable, e.g. `OPENAI_API_KEY`, or save it with `biorouter configure --provider \
     <provider> --secret-stdin`, which reads `NAME=value` lines from stdin.";

pub async fn handle_configure() -> anyhow::Result<()> {
    configure(Config::global(), super::needs_terminal::prompt_can_run()).await
}

/// `biorouter configure`, or with `--provider` (which clap allows only beside
/// `--secret-stdin`, and it beside `--provider`) the no-echo key entry of
/// [`handle_configure_provider_secret`].
pub async fn handle_configure_command(provider: Option<String>) -> anyhow::Result<()> {
    match provider {
        Some(provider) => handle_configure_provider_secret(provider).await,
        None => handle_configure().await,
    }
}

/// The most `biorouter configure --secret-stdin` reads from a pipe or a file
/// (issue #526). A provider's keys are a few hundred bytes; the bound keeps a
/// mistaken redirect (a log, a binary) from being read whole.
const SECRET_INPUT_MAX_BYTES: u64 = 64 * 1024;

/// How long the provider gets to answer the check before anything is saved.
const SECRET_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Why `--secret-stdin` does not run in an agent's shell.
const SECRET_ENTRY_IS_A_PERSONS: &str =
    "`biorouter configure --secret-stdin` enters a provider's key for a person, and this is an \
     agent's shell, so a key typed for it would pass through the model. Run it yourself, at a \
     terminal on this computer.";

/// Why `--secret-stdin` at a keyboard needs a terminal for its hidden prompt.
const SECRET_PROMPT_NEEDS_A_TERMINAL: &str =
    "`biorouter configure --secret-stdin` asks for the key at a hidden prompt, which needs a \
     terminal on stdin and stderr. Without one, send `NAME=value` lines on stdin instead, for \
     example from a file only you can read: `biorouter configure --provider <provider> \
     --secret-stdin < keys.env`.";

/// `biorouter configure --provider <provider> --secret-stdin` (issue #526):
/// save that provider's API key on this computer without it ever being echoed,
/// put on a command line, or kept in shell history.
///
/// At a terminal the key is asked for at a hidden prompt, one step after the
/// command, so there is no moment where a person pastes a key at the shell's own
/// prompt. Otherwise stdin is read as `NAME=value` lines (a file only the
/// account can read, or a secrets manager's output). Only the provider's own
/// secret settings are taken, never the ones that decide where a key is sent
/// (those stay with `biorouter configure` and, in a served page, the person's
/// proof, SD-18). The key is checked with the provider while it is held in
/// memory, and written only once the check passed; a refused, unreachable or
/// slow provider leaves the saved settings exactly as they were. The provider
/// and model choice is not changed.
pub async fn handle_configure_provider_secret(provider: String) -> anyhow::Result<()> {
    if super::needs_terminal::in_agent_shell() {
        super::needs_terminal::require(false, SECRET_ENTRY_IS_A_PERSONS)?;
    }
    let available = chat_providers().await;
    let metadata = available
        .iter()
        .map(|(metadata, _)| metadata)
        .find(|metadata| metadata.name == provider)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "`{provider}` is not a chat provider this build knows. `biorouter models \
                 providers` lists them."
            )
        })?;
    let accepted = secret_settings_taken(metadata)?;

    let mut typed = if std::io::stdin().is_terminal() {
        super::needs_terminal::require(
            super::needs_terminal::prompt_can_run(),
            SECRET_PROMPT_NEEDS_A_TERMINAL,
        )?;
        prompt_for_secret_settings(metadata, &accepted)?
    } else {
        let input = read_secret_input(std::io::stdin().lock())?;
        parse_secret_lines(&input, metadata, &accepted)?
    };

    let config = Config::global();
    let model = config
        .get_biorouter_provider()
        .ok()
        .filter(|current| *current == metadata.name)
        .and_then(|_| config.get_biorouter_model().ok())
        .unwrap_or_else(|| metadata.default_model.clone());
    let saved = save_checked_secrets(
        config,
        metadata,
        &typed,
        check_typed_credentials(&metadata.name, model),
    )
    .await;
    for setting in &mut typed {
        zeroize::Zeroize::zeroize(&mut setting.value);
    }
    let saved = saved?;

    for (name, home) in &saved {
        println!("Saved {name} for {} in {home}.", metadata.display_name);
    }
    println!(
        "These are the Biorouter settings in {}. A `biorouter serve` running as this account \
         reads the new key once it restarts.",
        Paths::config_dir().display()
    );
    if config.get_biorouter_provider().ok().as_deref() != Some(metadata.name.as_str()) {
        println!(
            "To use {}, choose it in the browser, or run `biorouter models set --provider {} \
             --model <model>`.",
            metadata.display_name, metadata.name
        );
    }
    Ok(())
}

/// The settings `--secret-stdin` takes for `metadata`: its secrets, other than
/// one a browser sign-in fills. Refused, with what to do instead, when there are
/// none.
fn secret_settings_taken(
    metadata: &biorouter::providers::base::ProviderMetadata,
) -> anyhow::Result<Vec<&biorouter::providers::base::ConfigKey>> {
    let taken: Vec<_> = metadata
        .config_keys
        .iter()
        .filter(|key| key.secret && !key.oauth_flow)
        .collect();
    if !taken.is_empty() {
        return Ok(taken);
    }
    if metadata.config_keys.iter().any(|key| key.oauth_flow) {
        anyhow::bail!(
            "{} signs in through a browser rather than with a key. Run `biorouter configure` at \
             a terminal to sign in.",
            metadata.display_name
        );
    }
    anyhow::bail!(
        "{} takes no API key. Choose it in the browser, or run `biorouter models set --provider \
         {} --model <model>`.",
        metadata.display_name,
        metadata.name
    )
}

/// Ask for each setting in `accepted` at a hidden prompt: the required ones, or
/// every one when none is required, in which case a blank answer skips it.
fn prompt_for_secret_settings(
    metadata: &biorouter::providers::base::ProviderMetadata,
    accepted: &[&biorouter::providers::base::ConfigKey],
) -> anyhow::Result<Vec<TypedSetting>> {
    let required: Vec<_> = accepted.iter().filter(|key| key.required).collect();
    let asked: Vec<_> = if required.is_empty() {
        accepted.iter().collect()
    } else {
        required
    };
    let optional = asked.iter().all(|key| !key.required);
    let mut typed = Vec::new();
    for key in asked {
        let mut prompt = cliclack::password(format!(
            "{} for {} (hidden as you type)",
            key.name, metadata.display_name
        ))
        .mask('▪');
        if optional {
            prompt = prompt.allow_empty();
        }
        let value = zeroize::Zeroizing::new(prompt.interact()?);
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if value.chars().any(char::is_control) {
            anyhow::bail!(
                "{} holds a control character, which no key does; nothing was saved.",
                key.name
            );
        }
        typed.push(TypedSetting {
            name: key.name.clone(),
            secret: true,
            value: value.to_string(),
        });
    }
    if typed.is_empty() {
        anyhow::bail!("No key was entered; nothing was saved.");
    }
    Ok(typed)
}

/// Read all of `input`, refusing more than [`SECRET_INPUT_MAX_BYTES`].
///
/// ⚠ A pipe whose writer never closes it is waited for: the bound is on size,
/// not time.
fn read_secret_input(input: impl std::io::Read) -> anyhow::Result<zeroize::Zeroizing<Vec<u8>>> {
    use std::io::Read;
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    input
        .take(SECRET_INPUT_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| anyhow::anyhow!("could not read the keys from stdin: {error}"))?;
    if bytes.len() as u64 > SECRET_INPUT_MAX_BYTES {
        anyhow::bail!(
            "stdin holds more than {} KiB, more than any provider's keys; nothing was saved.",
            SECRET_INPUT_MAX_BYTES / 1024
        );
    }
    Ok(bytes)
}

/// `NAME=value` lines, one per setting in `accepted`. Blank lines and lines
/// starting with `#` are skipped, and space around a name or a value is not
/// part of it.
///
/// ⚠ A refusal names a line by its number and never repeats what it held: a
/// line that is not a setting this provider takes may be a key with an `=` in
/// it.
fn parse_secret_lines(
    input: &[u8],
    metadata: &biorouter::providers::base::ProviderMetadata,
    accepted: &[&biorouter::providers::base::ConfigKey],
) -> anyhow::Result<Vec<TypedSetting>> {
    let names = accepted
        .iter()
        .map(|key| key.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let text = std::str::from_utf8(input)
        .map_err(|_| anyhow::anyhow!("stdin is not UTF-8 text; nothing was saved."))?;
    let mut typed: Vec<TypedSetting> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            anyhow::bail!(
                "line {number} of stdin is not NAME=value. {} takes {names}; nothing was saved.",
                metadata.display_name
            );
        };
        let Some(key) = accepted.iter().find(|key| key.name == name.trim()) else {
            anyhow::bail!(
                "line {number} of stdin names a setting this command does not take for {}. It \
                 takes {names}; nothing was saved.",
                metadata.display_name
            );
        };
        let value = value.trim();
        if value.is_empty() {
            anyhow::bail!(
                "line {number} of stdin gives {} no value; nothing was saved.",
                key.name
            );
        }
        if value.chars().any(char::is_control) {
            anyhow::bail!(
                "line {number} of stdin gives {} a control character, which no key holds; \
                 nothing was saved.",
                key.name
            );
        }
        if typed.iter().any(|setting| setting.name == key.name) {
            anyhow::bail!(
                "{} is given more than once on stdin; nothing was saved.",
                key.name
            );
        }
        typed.push(TypedSetting {
            name: key.name.clone(),
            secret: true,
            value: value.to_string(),
        });
    }
    if typed.is_empty() {
        anyhow::bail!(
            "stdin gave no key; nothing was saved. Send NAME=value lines: {} takes {names}.",
            metadata.display_name
        );
    }
    Ok(typed)
}

/// Why the provider did not accept the typed key, in words that never repeat
/// anything the provider sent back: an error from a provider can carry the
/// request's address, and some providers put the key in it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SecretCheckFailure {
    /// The provider could not be set up with the typed key and the saved
    /// settings. The text is the provider's own, with every typed value hidden.
    Unbuildable(String),
    Rejected,
    Unreachable,
    TimedOut,
}

impl SecretCheckFailure {
    fn sentence(&self, display_name: &str) -> String {
        match self {
            SecretCheckFailure::Unbuildable(reason) => format!(
                "{display_name} could not be set up with this key and the saved settings \
                 ({reason}). Set its other settings with `biorouter configure`; nothing was \
                 saved."
            ),
            SecretCheckFailure::Rejected => {
                format!("{display_name} rejected this key; nothing was saved.")
            }
            SecretCheckFailure::Unreachable => format!(
                "{display_name} could not be reached to check this key, so nothing was saved. \
                 Check the network and the provider's settings, then try again."
            ),
            SecretCheckFailure::TimedOut => format!(
                "{display_name} did not answer within {} seconds, so nothing was saved.",
                SECRET_CHECK_TIMEOUT.as_secs()
            ),
        }
    }
}

/// Build `provider` and ask it to check its credentials, with the typed values
/// in force (`with_config_overrides`, set by the caller) and none of them saved.
async fn check_typed_credentials(provider: &str, model: String) -> Result<(), SecretCheckFailure> {
    let model = ModelConfig::new(&model)
        .map_err(|error| SecretCheckFailure::Unbuildable(error.to_string()))?;
    let built = create(provider, model)
        .await
        .map_err(|error| SecretCheckFailure::Unbuildable(format!("{error:#}")))?;
    match tokio::time::timeout(SECRET_CHECK_TIMEOUT, built.check_credentials()).await {
        Err(_) => Err(SecretCheckFailure::TimedOut),
        Ok(Ok(())) => Ok(()),
        Ok(Err(biorouter::providers::errors::ProviderError::Authentication(_))) => {
            Err(SecretCheckFailure::Rejected)
        }
        Ok(Err(_)) => Err(SecretCheckFailure::Unreachable),
    }
}

/// `text` with every typed value replaced, for a message that quotes a
/// provider.
fn without_typed_values(text: &str, typed: &[TypedSetting]) -> String {
    typed
        .iter()
        .filter(|setting| !setting.value.is_empty())
        .fold(text.to_string(), |text, setting| {
            text.replace(&setting.value, "[hidden]")
        })
}

/// Run `check` with `typed` in force and nothing written, then save `typed`
/// once it passed, and say where each one went. Nothing is written when the
/// check fails or the saved settings cannot be read first; a write that fails
/// part-way puts back what was there before (`PreviousProviderSettings`).
async fn save_checked_secrets<F>(
    config: &'static Config,
    metadata: &biorouter::providers::base::ProviderMetadata,
    typed: &[TypedSetting],
    check: F,
) -> anyhow::Result<Vec<(String, String)>>
where
    F: std::future::Future<Output = Result<(), SecretCheckFailure>>,
{
    let display_name = metadata.display_name.as_str();
    if let Err(failure) =
        biorouter::config::with_config_overrides(typed_overrides(typed), check).await
    {
        let failure = match failure {
            SecretCheckFailure::Unbuildable(reason) => {
                SecretCheckFailure::Unbuildable(without_typed_values(&reason, typed))
            }
            other => other,
        };
        anyhow::bail!(failure.sentence(display_name));
    }

    let keys: Vec<_> = metadata
        .config_keys
        .iter()
        .filter(|key| typed.iter().any(|setting| setting.name == key.name))
        .cloned()
        .collect();
    let mut previous = PreviousProviderSettings::capture(config, &keys);
    if previous.keys.len() != keys.len() {
        previous.keep();
        anyhow::bail!(
            "the saved {display_name} settings could not be read, so nothing was saved: a key \
             that cannot be put back is not replaced."
        );
    }
    let file_before_write = std::env::var("BIOROUTER_DISABLE_KEYRING").is_ok();
    let mut saved = Vec::new();
    for setting in typed {
        let home = match config.set_secret(&setting.name, &setting.value) {
            Ok(()) => secret_home(file_before_write, false),
            Err(ConfigError::FallbackToFileStorage) => secret_home(file_before_write, true),
            Err(error) => {
                let restored = previous.restore();
                anyhow::bail!(
                    "{} could not be saved ({error}). {}",
                    setting.name,
                    match restored {
                        Ok(_) => format!("The saved {display_name} settings are as they were."),
                        Err(_) => format!(
                            "The saved {display_name} settings could not all be put back; run \
                             `biorouter configure` to check them."
                        ),
                    }
                );
            }
        };
        saved.push((setting.name.clone(), home));
    }
    previous.keep();
    Ok(saved)
}

/// [`handle_configure`] over an explicit config, so the refusal can be pinned
/// against a file a test owns rather than the process-global one.
///
/// ⚠ **The terminal check is the first statement**, ahead of the
/// `config.exists()` branch: both branches open with a cliclack prompt, and a
/// command that is about to decline must not have read or written anything on
/// the way there. Under a pipe this used to die inside the first prompt with
/// cliclack's bare `Error: not connected`.
async fn configure(config: &Config, terminal: bool) -> anyhow::Result<()> {
    super::needs_terminal::require(terminal, CONFIGURE_NEEDS_A_TERMINAL)?;

    if !config.exists() {
        handle_first_time_setup(config).await
    } else {
        handle_existing_config().await
    }
}

async fn handle_first_time_setup(config: &Config) -> anyhow::Result<()> {
    println!();
    println!(
        "{}",
        style("Welcome to biorouter! Let's get you set up.").dim()
    );
    println!(
        "{}",
        style("  you can rerun this command later to update your configuration").dim()
    );
    println!();
    cliclack::intro(style(" biorouter-configure ").on_cyan().black())?;

    let setup_method = cliclack::select("How would you like to set up your provider?")
        .item(
            "local",
            "Local Model (Llama Server)",
            "Run a private model on this computer. Free, no account or API key needed",
        )
        .item(
            "openrouter",
            "OpenRouter Login",
            "Sign in with OpenRouter to automatically configure models",
        )
        .item(
            "tetrate",
            "Tetrate Agent Router Service Login",
            "Sign in with Tetrate Agent Router Service to automatically configure models",
        )
        .item(
            "manual",
            "Manual Configuration",
            "Choose a provider and enter credentials manually",
        )
        .interact()?;

    match setup_method {
        "local" => {
            if let Err(e) = handle_local_llamacpp_setup(config).await {
                println!(
                    "\n  {} Local model setup failed: {} \n  Please try again or use manual configuration",
                    style("Error").red().italic(),
                    e,
                );
            }
        }
        "openrouter" => {
            if let Err(e) = handle_openrouter_auth().await {
                let _ = config.clear();
                println!(
                    "\n  {} OpenRouter authentication failed: {} \n  Please try again or use manual configuration",
                    style("Error").red().italic(),
                    e,
                );
            }
        }
        "tetrate" => {
            if let Err(e) = handle_tetrate_auth().await {
                let _ = config.clear();
                println!(
                    "\n  {} Tetrate Agent Router Service authentication failed: {} \n  Please try again or use manual configuration",
                    style("Error").red().italic(),
                    e,
                );
            }
        }
        "manual" => handle_manual_provider_setup(config).await,
        _ => unreachable!(),
    }
    Ok(())
}

/// Where a Llama Server catalog model comes from, for the picker: Hugging Face,
/// or an Ollama copy it can reuse. The Ollama tag named is a GGUF-only one when
/// the catalog has it: the bare tag (`qwen3.8:27b`) is a manifest list that
/// pulls an MLX build on Apple Silicon, which llama-server cannot load, so
/// naming it sent a Mac user to pull a copy that is never reused.
fn llamacpp_source_label(entry: &biorouter::providers::llamacpp::CatalogEntry) -> String {
    match entry
        .ollama_gguf_names
        .first()
        .copied()
        .or(entry.ollama_name)
    {
        Some(tag) => format!("Hugging Face, or Ollama {tag} if pulled"),
        None => "Hugging Face".to_string(),
    }
}

/// First-time "just give me a local model" path: pick from the curated
/// Llama Server catalog, then start the bundled llama-server (the first run
/// downloads the model from Hugging Face).
async fn handle_local_llamacpp_setup(config: &Config) -> anyhow::Result<()> {
    use biorouter::providers::llamacpp::{default_model_name, MODEL_CATALOG};
    use biorouter::providers::llamacpp_sidecar::LLAMACPP_DEFAULT_PORT;

    let default_model = default_model_name();
    let labels: Vec<String> = MODEL_CATALOG
        .iter()
        .map(|e| {
            format!(
                "{} · {} · {}",
                e.display_name,
                e.download_size,
                llamacpp_source_label(e)
            )
        })
        .collect();
    let mut select = cliclack::select("Choose a local model (downloaded on first use)")
        .initial_value(default_model);
    for (entry, label) in MODEL_CATALOG.iter().zip(&labels) {
        select = select.item(entry.name, label, entry.description);
    }
    let model = select.interact()?;

    // Explicitly persisting the (defaulted) port marks the provider configured.
    config.set_param("LLAMACPP_PORT", LLAMACPP_DEFAULT_PORT.to_string())?;

    let _ = cliclack::log::info(
        "The first run reuses a copy already on disk (the Hugging Face cache, or a GGUF pulled into Ollama); otherwise it downloads 18-19 GB, which can take a while.",
    );
    let spin = spinner();
    spin.start("Starting Llama Server and testing the model...");
    match test_provider_configuration("llamacpp", model, false, None).await {
        Ok(()) => {
            spin.stop(style("Llama Server is ready").green());
            config.set_biorouter_provider_and_model("llamacpp", model)?;
            print_config_file_saved()?;
            Ok(())
        }
        Err(e) => {
            spin.stop(style(e.to_string()).red());
            anyhow::bail!("local model test did not succeed")
        }
    }
}

async fn handle_manual_provider_setup(config: &Config) {
    match configure_provider_dialog().await {
        Ok(true) => {
            println!(
                "\n  {}: Run '{}' again to adjust your config or add extensions",
                style("Tip").green().italic(),
                style("biorouter configure").cyan()
            );
            set_extension(ExtensionEntry {
                enabled: true,
                config: ExtensionConfig::default(),
            });
        }
        Ok(false) => {
            let _ = config.clear();
            println!(
                "\n  {}: We did not save your config, inspect your credentials\n   and run '{}' again to ensure biorouter can connect",
                style("Warning").yellow().italic(),
                style("biorouter configure").cyan()
            );
        }
        Err(e) => {
            let _ = config.clear();
            print_manual_config_error(&e);
        }
    }
}

fn print_manual_config_error(e: &anyhow::Error) {
    match e.downcast_ref::<ConfigError>() {
        Some(ConfigError::NotFound(key)) => {
            println!(
                "\n  {} Required configuration key '{}' not found \n  Please provide this value and run '{}' again",
                style("Error").red().italic(),
                key,
                style("biorouter configure").cyan()
            );
        }
        Some(ConfigError::KeyringError(msg)) => {
            print_keyring_error(msg);
        }
        Some(ConfigError::DeserializeError(msg)) => {
            println!(
                "\n  {} Invalid configuration value: {} \n  Please check your input and run '{}' again",
                style("Error").red().italic(),
                msg,
                style("biorouter configure").cyan()
            );
        }
        Some(ConfigError::FileError(err)) => {
            println!(
                "\n  {} Failed to access config file: {} \n  Please check file permissions and run '{}' again",
                style("Error").red().italic(),
                err,
                style("biorouter configure").cyan()
            );
        }
        Some(ConfigError::DirectoryError(msg)) => {
            println!(
                "\n  {} Failed to access config directory: {} \n  Please check directory permissions and run '{}' again",
                style("Error").red().italic(),
                msg,
                style("biorouter configure").cyan()
            );
        }
        _ => {
            println!(
                "\n  {} {} \n  We did not save your config, inspect your credentials\n   and run '{}' again to ensure biorouter can connect",
                style("Error").red().italic(),
                e,
                style("biorouter configure").cyan()
            );
        }
    }
}

#[cfg(target_os = "macos")]
fn print_keyring_error(msg: &str) {
    println!(
        "\n  {} Failed to access secure storage (keyring): {} \n  Please check your system keychain and run '{}' again. \n  If your system is unable to use the keyring, please try setting secret key(s) via environment variables.",
        style("Error").red().italic(),
        msg,
        style("biorouter configure").cyan()
    );
}

#[cfg(target_os = "windows")]
fn print_keyring_error(msg: &str) {
    println!(
        "\n  {} Failed to access Windows Credential Manager: {} \n  Please check Windows Credential Manager and run '{}' again. \n  If your system is unable to use the Credential Manager, please try setting secret key(s) via environment variables.",
        style("Error").red().italic(),
        msg,
        style("biorouter configure").cyan()
    );
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn print_keyring_error(msg: &str) {
    println!(
        "\n  {} Failed to access secure storage: {} \n  Please check your system's secure storage and run '{}' again. \n  If your system is unable to use secure storage, please try setting secret key(s) via environment variables.",
        style("Error").red().italic(),
        msg,
        style("biorouter configure").cyan()
    );
}

async fn handle_existing_config() -> anyhow::Result<()> {
    let config_dir = Paths::config_dir().display().to_string();

    println!();
    println!(
        "{}",
        style("This will update your existing config files").dim()
    );
    println!(
        "{} {}",
        style("  if you prefer, you can edit them directly at").dim(),
        config_dir
    );
    println!();

    cliclack::intro(style(" biorouter-configure ").on_cyan().black())?;
    let action = cliclack::select("What would you like to configure?")
        .item(
            "providers",
            "Configure Providers",
            "Change provider or update credentials",
        )
        .item(
            "custom_providers",
            "Custom Providers",
            "Add custom provider with compatible API",
        )
        .item("add", "Add Extension", "Connect to a new extension")
        .item(
            "toggle",
            "Toggle Extensions",
            "Enable or disable connected extensions",
        )
        .item("remove", "Remove Extension", "Remove an extension")
        .item(
            "settings",
            "Biorouter settings",
            "Set the mode, tool output, tool permissions, experiments, the workflow GitHub repository and more",
        )
        .interact()?;

    match action {
        "toggle" => toggle_extensions_dialog(),
        "add" => configure_extensions_dialog(),
        "remove" => remove_extension_dialog(),
        "settings" => configure_settings_dialog().await,
        "providers" => configure_provider_dialog().await.map(|_| ()),
        "custom_providers" => configure_custom_provider_dialog(),
        _ => unreachable!(),
    }
}

/// Helper function to handle OAuth configuration for a provider
async fn handle_oauth_configuration(provider_name: &str, key_name: &str) -> anyhow::Result<()> {
    let _ = cliclack::log::info(format!(
        "Configuring {} using OAuth device code flow...",
        key_name
    ));

    // Create a temporary provider instance to handle OAuth
    let temp_model = ModelConfig::new("temp")?;
    match create(provider_name, temp_model).await {
        Ok(provider) => match provider.configure_oauth().await {
            Ok(_) => {
                let _ = cliclack::log::success("OAuth authentication completed successfully!");
                Ok(())
            }
            Err(e) => {
                let _ = cliclack::log::error(format!("Failed to authenticate: {}", e));
                Err(anyhow::anyhow!(
                    "OAuth authentication failed for {}: {}",
                    key_name,
                    e
                ))
            }
        },
        Err(e) => {
            let _ = cliclack::log::error(format!("Failed to create provider for OAuth: {}", e));
            Err(anyhow::anyhow!(
                "Failed to create provider for OAuth: {}",
                e
            ))
        }
    }
}

fn interactive_model_search(models: &[String]) -> anyhow::Result<String> {
    const MAX_VISIBLE: usize = 30;
    let mut query = String::new();

    loop {
        let _ = cliclack::clear_screen();

        let _ = cliclack::log::info(format!(
            "🔍 {} models available. Type to filter.",
            models.len()
        ));

        let input: String = cliclack::input("Filtering models, press Enter to search")
            .placeholder("e.g., gpt, sonnet, llama, qwen")
            .default_input(&query)
            .interact::<String>()?;
        query = input.trim().to_string();

        let filtered: Vec<String> = if query.is_empty() {
            models.to_vec()
        } else {
            let q = query.to_lowercase();
            models
                .iter()
                .filter(|m| m.to_lowercase().contains(&q))
                .cloned()
                .collect()
        };

        if filtered.is_empty() {
            let _ = cliclack::log::warning("No matching models. Try a different search.");
            continue;
        }

        let mut items: Vec<(String, String, &str)> = filtered
            .iter()
            .take(MAX_VISIBLE)
            .map(|m| (m.clone(), m.clone(), ""))
            .collect();

        if filtered.len() > MAX_VISIBLE {
            items.insert(
                0,
                (
                    "__refine__".to_string(),
                    format!(
                        "Refine search to see more (showing {} of {} results)",
                        MAX_VISIBLE,
                        filtered.len()
                    ),
                    "Too many matches",
                ),
            );
        } else {
            items.insert(
                0,
                (
                    "__new_search__".to_string(),
                    "Start a new search...".to_string(),
                    "Enter a different search term",
                ),
            );
        }

        let selection = cliclack::select("Select a model:")
            .items(&items)
            .interact()?;

        if selection == "__refine__" {
            continue;
        } else if selection == "__new_search__" {
            query.clear();
            continue;
        } else {
            return Ok(selection);
        }
    }
}

const MAX_MODELS: usize = 10;

/// The models `configure` offers first, each with the hint shown beside it.
///
/// For Ollama these are the pulled tags of Biorouter's two supported local
/// models ([`OLLAMA_RECOMMENDED_MODELS`], default first), a quantization
/// variant of either included (`qwen3.8:27b-q4_K_M`). Ollama declares no known
/// models (its list is whatever the user pulled), so before this the dialog
/// listed the pulled tags alphabetically and pre-selected the first: on a
/// machine holding `deepseek-r1:8b` and `qwen3.8:27b`, Enter chose a model that
/// never calls tools. Coding-agent account catalogues keep their default-first order.
/// Other providers use their known models that the listing returned.
///
/// ⚠ A cloud tag is never a variant. `gemma4:31b-cloud` starts with
/// `gemma4:31b-`, and it is a real Ollama cloud model: Ollama forwards it to
/// ollama.com. It used to be offered here as "Recommended: Gemma 4 31B",
/// pre-selected, with the "neither is pulled" advice suppressed, so Enter saved
/// a model that sends every prompt off the machine under the label of the
/// supported local one.
///
/// [`OLLAMA_RECOMMENDED_MODELS`]: biorouter::providers::ollama::OLLAMA_RECOMMENDED_MODELS
fn suggested_models(
    provider_name: &str,
    models: &[String],
    known_models: &[String],
) -> Vec<(String, String)> {
    if matches!(provider_name, "codex" | "github_copilot") {
        return models
            .iter()
            .take(MAX_MODELS)
            .enumerate()
            .map(|(index, model)| {
                (
                    model.clone(),
                    if index == 0 { "Recommended" } else { "" }.to_string(),
                )
            })
            .collect();
    }
    if provider_name == "ollama" {
        use biorouter::providers::ollama::OLLAMA_RECOMMENDED_MODELS;
        let mut suggested = Vec::new();
        for recommended in OLLAMA_RECOMMENDED_MODELS {
            let variant_prefix = format!("{}-", recommended.tag);
            let hint = format!("Recommended: {}", recommended.display_name);
            suggested.extend(
                models
                    .iter()
                    .filter(|m| *m == recommended.tag)
                    .chain(models.iter().filter(|m| m.starts_with(&variant_prefix)))
                    .filter(|m| !is_ollama_cloud_model(m))
                    .map(|m| (m.clone(), hint.clone())),
            );
        }
        return suggested;
    }
    known_models
        .iter()
        .filter(|name| models.contains(name))
        .map(|name| (name.clone(), "Recommended".to_string()))
        .collect()
}

/// What `configure` says when this Ollama has neither supported model pulled:
/// which two to pull, and why choosing another is a gamble.
fn ollama_recommendation_note(models: &[String]) -> Option<String> {
    use biorouter::providers::ollama::OLLAMA_RECOMMENDED_MODELS;
    if !suggested_models("ollama", models, &[]).is_empty() {
        return None;
    }
    let pair = OLLAMA_RECOMMENDED_MODELS
        .iter()
        .map(|m| {
            format!(
                "{} ({}, about {} GB)",
                m.tag, m.display_name, m.approx_download_gb
            )
        })
        .collect::<Vec<_>>()
        .join(" and ");
    let default = OLLAMA_RECOMMENDED_MODELS
        .first()
        .map(|m| m.tag)
        .unwrap_or(biorouter::providers::ollama::OLLAMA_DEFAULT_MODEL);
    Some(format!(
        "Biorouter supports two local models on Ollama: {pair}. Neither is pulled on this \
         Ollama yet. Run `ollama pull {default}`, then run `biorouter configure` again. Other \
         models may write tool calls as text instead of making them."
    ))
}

/// The value of the "Enter a model not listed..." row in every model list.
const UNLISTED_MODEL_KEY: &str = "__unlisted__";

/// The row that lets a person type a model the list does not name.
fn unlisted_model_item() -> (String, String, String) {
    (
        UNLISTED_MODEL_KEY.to_string(),
        "Enter a model not listed...".to_string(),
        String::new(),
    )
}

/// The models `configure` offers for a provider whose listing named nothing
/// (`fetch_recommended_models` answered `Ok(None)`: the Versa gateways, Amazon
/// Bedrock, Azure OpenAI and the others with no live model list). They are the
/// models the provider declares, the list `biorouter models list` prints, with
/// its default first and marked Recommended, then a row to type any other name.
/// `None` when it declares none, and the dialog asks for a typed name as before.
///
/// Phase C, T1 F3: this path asked for a typed name with the default filled in,
/// so Versa API Bedrock never showed Sonnet 5.5 or Haiku 5.5 although
/// `biorouter models list versa_bedrock` named both. The typed-name row is
/// offered whatever `allows_unlisted_models` says, because this path always
/// accepted a typed name: a gateway can enable a model before Biorouter lists it.
fn known_model_items(
    provider_meta: &biorouter::providers::base::ProviderMetadata,
) -> Option<Vec<(String, String, String)>> {
    if provider_meta.known_models.is_empty() {
        return None;
    }
    let default = provider_meta.default_model.as_str();
    let mut items = vec![(
        default.to_string(),
        default.to_string(),
        "Recommended".to_string(),
    )];
    items.extend(
        provider_meta
            .known_models
            .iter()
            .filter(|model| model.name != default)
            .map(|model| (model.name.clone(), model.name.clone(), String::new())),
    );
    items.push(unlisted_model_item());
    Some(items)
}

/// The row a model list opens on: `current` when the list offers it. `None`
/// opens it on its first row.
fn preselected_model(items: &[(String, String, String)], current: Option<&str>) -> Option<String> {
    let current = current?;
    items
        .iter()
        .map(|(value, _, _)| value)
        .find(|value| *value == current && *value != UNLISTED_MODEL_KEY)
        .cloned()
}

/// Ask for one of `items`, opening on `initial` when given; the unlisted row
/// asks for a typed name.
fn pick_model(
    items: &[(String, String, String)],
    initial: Option<String>,
    provider_meta: &biorouter::providers::base::ProviderMetadata,
) -> anyhow::Result<String> {
    let mut select = cliclack::select("Select a model:").items(items);
    if let Some(initial) = initial {
        select = select.initial_value(initial);
    }
    let selection = select.interact()?;
    if selection == UNLISTED_MODEL_KEY {
        prompt_unlisted_model(provider_meta)
    } else {
        Ok(selection)
    }
}

/// `items` with `current`, the model in use, offered as a row of its own after
/// the default when the list does not name it (a deployment the person typed
/// before), so the list can open on it and Enter keeps it.
fn with_model_in_use(
    mut items: Vec<(String, String, String)>,
    current: Option<&str>,
) -> Vec<(String, String, String)> {
    let Some(current) = current.map(str::trim).filter(|current| !current.is_empty()) else {
        return items;
    };
    if current == UNLISTED_MODEL_KEY || items.iter().any(|(value, _, _)| value == current) {
        return items;
    }
    let at = items.len().min(1);
    items.insert(
        at,
        (
            current.to_string(),
            current.to_string(),
            "In use".to_string(),
        ),
    );
    items
}

/// The model prompt for a provider whose listing named nothing: the models it
/// declares ([`known_model_items`]), opening on `current`, the model in use,
/// or, when it declares none, a typed name under `typed_prompt` with
/// `typed_default` filled in.
fn prompt_for_unlisted_provider_model(
    provider_meta: &biorouter::providers::base::ProviderMetadata,
    current: Option<&str>,
    typed_prompt: &str,
    typed_default: &str,
) -> anyhow::Result<String> {
    match known_model_items(provider_meta) {
        Some(items) => {
            let items = with_model_in_use(items, current);
            let initial = preselected_model(&items, current.map(str::trim));
            pick_model(&items, initial, provider_meta)
        }
        None => Ok(cliclack::input(typed_prompt)
            .default_input(typed_default)
            .interact()?),
    }
}

/// Ask for one of a provider's listed `models`, opening on `current`, the model
/// in use, when the list shows it.
fn select_model_from_list(
    models: &[String],
    provider_meta: &biorouter::providers::base::ProviderMetadata,
    current: Option<&str>,
) -> anyhow::Result<String> {
    let known_models: Vec<String> = provider_meta
        .known_models
        .iter()
        .map(|m| m.name.clone())
        .collect();
    let suggested = suggested_models(&provider_meta.name, models, &known_models);
    if provider_meta.name == "ollama" {
        if let Some(note) = ollama_recommendation_note(models) {
            let _ = cliclack::log::info(note);
        }
    }

    // Smart model selection:
    // If we have more than MAX_MODELS models, show the recommended models with additional search option.
    // Otherwise, show all models without search.
    if models.len() > MAX_MODELS {
        if !suggested.is_empty() {
            let (mut model_items, initial) =
                long_model_list(&provider_meta.name, models, &known_models, current);

            if provider_meta.allows_unlisted_models {
                model_items.push(unlisted_model_item());
            }

            let mut select = cliclack::select("Select a model:").items(&model_items);
            if let Some(initial) = initial {
                select = select.initial_value(initial);
            }
            let selection = select.interact()?;

            if selection == "search_all" {
                Ok(interactive_model_search(models)?)
            } else if selection == UNLISTED_MODEL_KEY {
                prompt_unlisted_model(provider_meta)
            } else {
                Ok(selection)
            }
        } else {
            Ok(interactive_model_search(models)?)
        }
    } else {
        let (mut model_items, initial) =
            short_model_selection(&provider_meta.name, models, &suggested, current);

        if provider_meta.allows_unlisted_models {
            model_items.push(unlisted_model_item());
        }

        pick_model(&model_items, initial, provider_meta)
    }
}

fn long_model_list(
    provider_name: &str,
    models: &[String],
    known_models: &[String],
    current: Option<&str>,
) -> (Vec<(String, String, String)>, Option<String>) {
    let mut items: Vec<(String, String, String)> =
        suggested_models(provider_name, models, known_models)
            .into_iter()
            .map(|(model, hint)| (model.clone(), model, hint))
            .collect();
    items.insert(
        if matches!(provider_name, "codex" | "github_copilot") {
            items.len()
        } else {
            0
        },
        (
            "search_all".to_string(),
            "Search all models...".to_string(),
            "Search complete model list".to_string(),
        ),
    );
    if matches!(provider_name, "codex" | "github_copilot") {
        items = with_model_in_use(items, current);
    }
    let initial = preselected_model(&items, current);
    (items, initial)
}

fn short_model_selection(
    provider_name: &str,
    models: &[String],
    suggested: &[(String, String)],
    current: Option<&str>,
) -> (Vec<(String, String, String)>, Option<String>) {
    let items = short_model_list(provider_name, models, suggested);
    let items = if matches!(provider_name, "codex" | "github_copilot") {
        with_model_in_use(items, current)
    } else {
        items
    };
    let initial = preselected_model(&items, current);
    (items, initial)
}

/// The hint beside an Ollama cloud model in the list.
const OLLAMA_CLOUD_MODEL_HINT: &str = "Cloud model: Ollama sends it to ollama.com (public)";

/// The whole list, when it is short enough to show without a search. For
/// Ollama the supported models come first, so the pre-selected first item (what
/// Enter chooses) is one of them, and cloud models come last with a hint saying
/// where they run: with neither supported model pulled, Enter would otherwise
/// choose whatever sorts first, `gemma4:31b-cloud` ahead of `llama3.1:8b`.
/// Other providers keep the listing's order.
fn short_model_list(
    provider_name: &str,
    models: &[String],
    suggested: &[(String, String)],
) -> Vec<(String, String, String)> {
    if provider_name != "ollama" {
        return models
            .iter()
            .map(|m| (m.clone(), m.clone(), String::new()))
            .collect();
    }
    let rest = models
        .iter()
        .filter(|m| !suggested.iter().any(|(p, _)| p == *m));
    let (cloud, local): (Vec<&String>, Vec<&String>) = rest.partition(|m| is_ollama_cloud_model(m));
    suggested
        .iter()
        .map(|(m, hint)| (m.clone(), m.clone(), hint.clone()))
        .chain(
            local
                .into_iter()
                .map(|m| (m.clone(), m.clone(), String::new())),
        )
        .chain(
            cloud
                .into_iter()
                .map(|m| (m.clone(), m.clone(), OLLAMA_CLOUD_MODEL_HINT.to_string())),
        )
        .collect()
}

fn prompt_unlisted_model(
    provider_meta: &biorouter::providers::base::ProviderMetadata,
) -> anyhow::Result<String> {
    let model: String = cliclack::input("Enter the model name:")
        .placeholder(&provider_meta.default_model)
        .validate(|input: &String| {
            if input.trim().is_empty() {
                Err("Please enter a model name")
            } else {
                Ok(())
            }
        })
        .interact()?;
    Ok(model.trim().to_string())
}

/// W2-PRV-2 — a provider's settings as they were before `configure` changed
/// them. `configure` stores each key as it is typed and only then asks the
/// provider (a model listing, then one tool call), so a key the provider
/// rejected had already replaced the working one. Captured before the first
/// write, and put back when the provider refuses what was typed.
struct PreviousProviderSettings {
    config: &'static Config,
    /// `(name, secret, stored value)`. A setting whose store could not be read
    /// at capture is left out: restoring from a failed read would delete a key
    /// that was there all along.
    keys: Vec<(String, bool, Option<Value>)>,
    /// Put the settings back if the dialog is left without [`Self::keep`]:
    /// an error or an interrupt part-way through leaves nothing half-replaced.
    armed: bool,
}

impl PreviousProviderSettings {
    /// The stored value of every setting in `keys`, from the config file and
    /// the secret store only: an environment variable is not a saved value.
    fn capture(config: &'static Config, keys: &[biorouter::providers::base::ConfigKey]) -> Self {
        let values = config.all_values().ok();
        let secrets = if keys.iter().any(|key| key.secret) {
            config.all_secrets().ok()
        } else {
            Some(HashMap::new())
        };
        Self {
            config,
            keys: keys
                .iter()
                .filter_map(|key| {
                    let store = if key.secret {
                        secrets.as_ref()
                    } else {
                        values.as_ref()
                    }?;
                    Some((key.name.clone(), key.secret, store.get(&key.name).cloned()))
                })
                .collect(),
            armed: true,
        }
    }

    /// The new settings stand: nothing will be put back.
    fn keep(&mut self) {
        self.armed = false;
    }

    /// Put back every setting that changed since [`Self::capture`], removing
    /// one that did not exist then. Returns whether anything was put back.
    fn restore(&mut self) -> anyhow::Result<bool> {
        self.armed = false;
        let config = self.config;
        let values = config.all_values()?;
        let secrets = if self.keys.iter().any(|(_, secret, _)| *secret) {
            config.all_secrets()?
        } else {
            HashMap::new()
        };
        let mut restored = false;
        for (name, secret, previous) in &self.keys {
            let now = if *secret {
                secrets.get(name)
            } else {
                values.get(name)
            };
            if now == previous.as_ref() {
                continue;
            }
            let result = match previous {
                Some(value) => config.set(name, value, *secret),
                None if *secret => config.delete_secret(name),
                None => config.delete(name),
            };
            match result {
                Ok(()) | Err(ConfigError::FallbackToFileStorage) => restored = true,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(restored)
    }

    /// [`Self::restore`], said to the person, after the provider refused.
    /// `typed_something` says whether values were typed, which since T3-SH-5
    /// are never written before the check and so need no putting back.
    fn restore_after_refusal(
        &mut self,
        display_name: &str,
        typed_something: bool,
    ) -> anyhow::Result<()> {
        if self.restore()? || typed_something {
            let _ = cliclack::log::info(format!(
                "Your previous {display_name} settings were kept; nothing you typed was saved."
            ));
        }
        Ok(())
    }
}

impl Drop for PreviousProviderSettings {
    fn drop(&mut self) {
        if self.armed {
            if let Err(error) = self.restore() {
                tracing::warn!("could not put the previous provider settings back: {error}");
            }
        }
    }
}

fn try_store_secret(config: &Config, key_name: &str, value: String) -> anyhow::Result<bool> {
    Ok(store_secret_reporting(config, key_name, value)?.is_some())
}

/// Where a secret [`try_store_secret`] stored ended up: the system keychain, or
/// the private secrets file when there is no usable keychain (a headless Linux
/// host, `BIOROUTER_DISABLE_KEYRING`, or a keychain that refused the write).
///
/// T3-SH-6. `configure` asked "save this value to your keyring?" and then said
/// "Saved KEY to <config.yaml>" for a secret, on a host where it had gone to
/// `secrets.yaml` and `config.yaml` held nothing of it.
fn secret_home(file_before_write: bool, fell_back: bool) -> String {
    if file_before_write || fell_back {
        format!(
            "the private file {}",
            Paths::config_dir().join("secrets.yaml").display()
        )
    } else {
        "the system keychain".to_string()
    }
}

/// [`try_store_secret`], and where the secret went, for the confirmation line.
/// `None` when it could not be stored (the reason has been said).
fn store_secret_reporting(
    config: &Config,
    key_name: &str,
    value: String,
) -> anyhow::Result<Option<String>> {
    let file_before_write = std::env::var("BIOROUTER_DISABLE_KEYRING").is_ok();
    match config.set_secret(key_name, &value) {
        Ok(()) => Ok(Some(secret_home(file_before_write, false))),
        Err(ConfigError::FallbackToFileStorage) => Ok(Some(secret_home(file_before_write, true))),
        Err(e) => {
            cliclack::outro(style(format!(
                "Failed to store {} securely: {}. Please ensure your system's secure storage is accessible. Alternatively you can run with BIOROUTER_DISABLE_KEYRING=true or set the key in your environment variables",
                key_name, e
            )).on_red().white())?;
            Ok(None)
        }
    }
}

/// T3-SH-5 — a value typed for a provider in `configure`, held in memory until
/// the provider has accepted it.
///
/// `configure` used to store each value as it was typed and check the provider
/// afterwards. A refusal, Esc or Ctrl-C put the old values back, but a SIGTERM
/// or SIGHUP (closing the terminal) at "Enter a model from that provider" left
/// the wrong key saved and the working one gone. Nothing typed is written now
/// until the provider has answered the check with it.
#[derive(Debug, Clone, PartialEq)]
struct TypedSetting {
    name: String,
    secret: bool,
    value: String,
}

/// The typed values as the task-local overrides the provider is built and
/// checked under, the form `/config/check_provider` uses: keys upper-cased, a
/// secret as a JSON string literal so an all-digit key stays a string.
fn typed_overrides(typed: &[TypedSetting]) -> HashMap<String, String> {
    typed
        .iter()
        .map(|setting| {
            let value = if setting.secret {
                serde_json::to_string(&setting.value).unwrap_or_else(|_| setting.value.clone())
            } else {
                setting.value.clone()
            };
            (setting.name.to_uppercase(), value)
        })
        .collect()
}

/// Write the typed values once the provider has accepted them: settings
/// first, credentials last (a failed setting stops before any key is written),
/// saying where each one went. `Ok(false)` when a secret could not be stored.
fn save_typed_settings(config: &Config, typed: &[TypedSetting]) -> anyhow::Result<bool> {
    for setting in typed.iter().filter(|setting| !setting.secret) {
        config.set_param(&setting.name, &setting.value)?;
        let _ = cliclack::log::info(format!("Saved {} to {}", setting.name, config.path()));
    }
    for setting in typed.iter().filter(|setting| setting.secret) {
        match store_secret_reporting(config, &setting.name, setting.value.clone())? {
            Some(home) => {
                let _ = cliclack::log::info(format!("Saved {} in {home}", setting.name));
            }
            None => return Ok(false),
        }
    }
    Ok(true)
}

/// Run `check` with the typed values in force but not written, and write them
/// only if it passes. Returns the check's own result; on `Err` nothing typed
/// has been written.
async fn check_then_save<T, F>(
    config: &Config,
    typed: &[TypedSetting],
    check: F,
) -> anyhow::Result<Option<T>>
where
    F: std::future::Future<Output = anyhow::Result<T>>,
{
    let checked = biorouter::config::with_config_overrides(typed_overrides(typed), check).await?;
    if !save_typed_settings(config, typed)? {
        return Ok(None);
    }
    Ok(Some(checked))
}

/// The providers `configure` offers for a chat: every registered one but the
/// decision providers, as the desktop's provider catalog does. A decision
/// model answers a question with probabilities and runs no agent loop, so it
/// can never pass the chat check at the end of the setup, and as a lead model
/// it would make every later chat a decision chat. A decision model is bound
/// with `biorouter models set --provider ollama_decision --model <model>`.
async fn chat_providers() -> Vec<(
    biorouter::providers::base::ProviderMetadata,
    biorouter::providers::base::ProviderType,
)> {
    let mut available = providers().await;
    available.retain(|(meta, _)| !is_decision_chat_provider(&meta.name));
    available
}

#[allow(clippy::too_many_lines)]
pub async fn configure_provider_dialog() -> anyhow::Result<bool> {
    // Get global config instance
    let config = Config::global();

    // Get the chat providers and their metadata.
    let mut available_providers = chat_providers().await;

    // Order as the GUI does: local models first (Llama Server, then Ollama),
    // institutional second, everything else alphabetically by display name.
    fn provider_sort_key(meta: &biorouter::providers::base::ProviderMetadata) -> (u8, u8, String) {
        let (group, priority) = match meta.name.as_str() {
            "llamacpp" => (0u8, 0u8),
            "ollama" => (0, 1),
            "versa_azure" => (1, 0),
            "versa_bedrock" => (1, 1),
            _ => (2, 0),
        };
        (group, priority, meta.display_name.clone())
    }
    available_providers.sort_by_key(|(meta, _)| provider_sort_key(meta));

    // Create selection items from provider metadata
    let provider_items: Vec<(&String, &str, &str)> = available_providers
        .iter()
        .map(|(p, _)| (&p.name, p.display_name.as_str(), p.description.as_str()))
        .collect();

    // Get current default provider if it exists
    let current_provider: Option<String> = config.get_biorouter_provider().ok();
    let default_provider = current_provider.unwrap_or_default();

    // Select provider
    let provider_name = cliclack::select("Which model provider should we use?")
        .initial_value(&default_provider)
        .items(&provider_items)
        .interact()?;

    // Get the selected provider's metadata
    let (provider_meta, _) = available_providers
        .iter()
        .find(|(p, _)| &p.name == provider_name)
        .expect("Selected provider must exist in metadata");

    // Issue #56, DR-17 requirement 3. Before the keys are collected, so the user
    // reads it while they can still pick something else — and unconditionally on
    // the master privacy switch, which turns off enforcement and not the truth.
    print_non_private_model_disclosure(provider_meta)?;

    // W2-PRV-2: before the first write, so a key the provider refuses below
    // never replaces the one that worked. Typed values are no longer written
    // before the check (T3-SH-5, `TypedSetting`); what this still guards is an
    // OAuth sign-in, which stores its own tokens as it completes.
    let mut previous = PreviousProviderSettings::capture(config, &provider_meta.config_keys);
    let mut typed: Vec<TypedSetting> = Vec::new();

    // Configure required provider keys
    for key in &provider_meta.config_keys {
        if !key.required {
            continue;
        }

        // First check if the value is set via environment variable
        let from_env = std::env::var(&key.name).ok();

        match from_env {
            Some(env_value) => {
                let _ =
                    cliclack::log::info(format!("{} is set via environment variable", key.name));
                // T3-SH-6: where a value is kept depends on the host (the
                // system keychain, or a private file where there is none), and
                // the confirmation after the check names the place it went.
                if cliclack::confirm("Save this value in Biorouter's settings?")
                    .initial_value(true)
                    .interact()?
                {
                    typed.push(TypedSetting {
                        name: key.name.clone(),
                        secret: key.secret,
                        value: env_value,
                    });
                }
            }
            None => {
                let existing: Result<String, _> = if key.secret {
                    config.get_secret(&key.name)
                } else {
                    config.get_param(&key.name)
                };

                match existing {
                    Ok(_) => {
                        let _ = cliclack::log::info(format!("{} is already configured", key.name));
                        if cliclack::confirm("Would you like to update this value?").interact()? {
                            // Check if this key uses OAuth flow
                            if key.oauth_flow {
                                handle_oauth_configuration(provider_name, &key.name).await?;
                            } else {
                                // Non-OAuth key, use manual entry
                                let value: String = if key.secret {
                                    cliclack::password(format!("Enter new value for {}", key.name))
                                        .mask('▪')
                                        .interact()?
                                } else {
                                    let mut input = cliclack::input(format!(
                                        "Enter new value for {}",
                                        key.name
                                    ));
                                    if key.default.is_some() {
                                        input = input.default_input(&key.default.clone().unwrap());
                                    }
                                    input.interact()?
                                };

                                typed.push(TypedSetting {
                                    name: key.name.clone(),
                                    secret: key.secret,
                                    value,
                                });
                            }
                        }
                    }
                    Err(_) => {
                        if key.oauth_flow {
                            handle_oauth_configuration(provider_name, &key.name).await?;
                        } else {
                            // Non-OAuth key, use manual entry
                            let value: String = if key.secret {
                                cliclack::password(format!(
                                    "Provider {} requires {}, please enter a value",
                                    provider_meta.display_name, key.name
                                ))
                                .mask('▪')
                                .interact()?
                            } else {
                                let mut input = cliclack::input(format!(
                                    "Provider {} requires {}, please enter a value",
                                    provider_meta.display_name, key.name
                                ));
                                if key.default.is_some() {
                                    input = input.default_input(&key.default.clone().unwrap());
                                }
                                input.interact()?
                            };

                            typed.push(TypedSetting {
                                name: key.name.clone(),
                                secret: key.secret,
                                value,
                            });
                        }
                    }
                }
            }
        }
    }

    // Offer the provider's optional settings too — the GUI renders every
    // config key for a provider, so mirror that here without forcing users
    // through fields they don't need.
    let optional_keys: Vec<_> = provider_meta
        .config_keys
        .iter()
        .filter(|k| !k.required)
        .collect();
    if !optional_keys.is_empty() {
        let mut select = cliclack::multiselect(format!(
            "Optional {} settings to configure (space to toggle, enter to skip/continue)",
            provider_meta.display_name
        ))
        .required(false);
        for key in &optional_keys {
            let hint = if key.secret {
                if config.get_secret::<String>(&key.name).is_ok() {
                    "configured".to_string()
                } else {
                    "not set".to_string()
                }
            } else {
                config
                    .get_param::<String>(&key.name)
                    .ok()
                    .or_else(|| key.default.clone())
                    .unwrap_or_else(|| "not set".to_string())
            };
            select = select.item(key.name.clone(), key.name.clone(), hint);
        }
        let chosen: Vec<String> = select.interact()?;
        for key_name in chosen {
            let key = optional_keys
                .iter()
                .find(|k| k.name == key_name)
                .expect("chosen key must exist");
            let value: String = if key.secret {
                cliclack::password(format!("Enter value for {}", key.name))
                    .mask('▪')
                    .interact()?
            } else {
                let mut input = cliclack::input(format!("Enter value for {}", key.name));
                let prefill = config
                    .get_param::<String>(&key.name)
                    .ok()
                    .or_else(|| key.default.clone());
                if let Some(prefill) = prefill {
                    input = input.default_input(&prefill);
                }
                input.interact()?
            };
            typed.push(TypedSetting {
                name: key.name.clone(),
                secret: key.secret,
                value,
            });
        }
    }

    // Everything below runs with the typed values in force and none of them
    // written: the provider is built, listed and tried with them, and they are
    // saved only once that passed (`check_then_save`).
    let overrides = typed_overrides(&typed);
    let nothing_typed_was_saved = !typed.is_empty();
    let spin = spinner();
    spin.start("Attempting to fetch supported models...");
    let temp_model_config = ModelConfig::new(&provider_meta.default_model)?;
    let temp_provider = match biorouter::config::with_config_overrides(
        overrides.clone(),
        create(provider_name, temp_model_config),
    )
    .await
    {
        Ok(provider) => provider,
        Err(error) => {
            spin.stop(style("The provider could not be set up").red());
            previous.restore_after_refusal(&provider_meta.display_name, nothing_typed_was_saved)?;
            return Err(error);
        }
    };
    let models_res = biorouter::config::with_config_overrides(
        overrides.clone(),
        retry_operation(&RetryConfig::default(), || async {
            temp_provider.fetch_recommended_models().await
        }),
    )
    .await;
    // A provider with no model listing (the Versa gateways) sent nothing above,
    // so ask it the way it can be asked before the model prompt: a key it
    // refuses is said now, not after a model has been chosen (T3-SH-3).
    let models_res = match models_res {
        // Bounded, as the daemon's check is: a gateway that does not answer
        // says nothing about the key, and the model prompt follows.
        Ok(None) => match tokio::time::timeout(
            std::time::Duration::from_secs(10),
            biorouter::config::with_config_overrides(overrides, temp_provider.check_credentials()),
        )
        .await
        {
            Ok(Err(biorouter::providers::errors::ProviderError::Authentication(reason))) => Err(
                biorouter::providers::errors::ProviderError::Authentication(format!(
                    "{} rejected these credentials: {}",
                    provider_meta.display_name,
                    reason.trim()
                )),
            ),
            _ => Ok(None),
        },
        other => other,
    };
    spin.stop(style("Model fetch complete").green());

    // Select a model: on fetch error show styled error and abort; if Some(models), show list;
    // if None, the provider's declared models (else a typed name). Either list opens on the
    // model in use when it is this provider's.
    let env_model = std::env::var("BIOROUTER_MODEL").ok();
    let current = env_model.clone().or_else(|| {
        config
            .get_biorouter_provider()
            .ok()
            .filter(|current| current == provider_name)
            .and_then(|_| config.get_biorouter_model().ok())
    });
    let model: String = match models_res {
        Err(e) => {
            // Provider hook error
            previous.restore_after_refusal(&provider_meta.display_name, nothing_typed_was_saved)?;
            cliclack::outro(style(e.to_string()).on_red().white())?;
            return Ok(false);
        }
        Ok(Some(models)) => select_model_from_list(&models, provider_meta, current.as_deref())?,
        Ok(None) => {
            let default_model = env_model.unwrap_or(provider_meta.default_model.clone());
            prompt_for_unlisted_provider_model(
                provider_meta,
                current.as_deref(),
                "Enter a model from that provider:",
                &default_model,
            )?
        }
    };

    // Test the configuration
    let spin = spinner();
    spin.start("Checking your configuration...");

    let toolshim_enabled = std::env::var("BIOROUTER_TOOLSHIM")
        .map(|val| val == "1" || val.to_lowercase() == "true")
        .unwrap_or(false);
    let toolshim_model = std::env::var("BIOROUTER_TOOLSHIM_OLLAMA_MODEL").ok();

    match check_then_save(
        config,
        &typed,
        test_provider_configuration(provider_name, &model, toolshim_enabled, toolshim_model),
    )
    .await
    {
        Ok(Some(())) => {
            previous.keep();
            config.set_biorouter_provider_and_model(provider_name, &model)?;
            print_config_file_saved()?;
            Ok(true)
        }
        // A setting was saved but a key could not be; the guard puts the
        // settings back, and the reason has been said.
        Ok(None) => Ok(false),
        Err(e) => {
            spin.stop(style(e.to_string()).red());
            previous.restore_after_refusal(&provider_meta.display_name, nothing_typed_was_saved)?;
            cliclack::outro(style("Failed to configure provider: init chat completion request with tool did not succeed.").on_red().white())?;
            Ok(false)
        }
    }
}

/// Configure capabilities and extensions that can be used with biorouter.
/// The storage layer retains the historical extension-shaped representation,
/// but the user-facing distinction must remain explicit.
pub fn toggle_extensions_dialog() -> anyhow::Result<()> {
    for warning in biorouter::config::get_warnings() {
        eprintln!("{}", style(format!("Warning: {}", warning)).yellow());
    }

    let extensions = get_all_extensions();

    if extensions.is_empty() {
        cliclack::outro("No capabilities or extensions are configured yet.")?;
        return Ok(());
    }

    // Create a list of extension names and their enabled status
    let mut extension_status: Vec<(String, bool)> = extensions
        .iter()
        .map(|entry| (entry.config.name().to_string(), entry.enabled))
        .collect();

    // Sort extensions alphabetically by name
    extension_status.sort_by(|a, b| a.0.cmp(&b.0));

    // Get currently enabled extensions for the selection
    let enabled_extensions: Vec<&String> = extension_status
        .iter()
        .filter(|(_, enabled)| *enabled)
        .map(|(name, _)| name)
        .collect();

    // Let the user toggle both categories while preserving their classification.
    let selected = cliclack::multiselect(
        "Enable capabilities and extensions: (use \"space\" to toggle and \"enter\" to submit)",
    )
    .required(false)
    .items(
        &extension_status
            .iter()
            .map(|(name, _)| (name, name.as_str(), MULTISELECT_VISIBILITY_HINT))
            .collect::<Vec<_>>(),
    )
    .initial_values(enabled_extensions)
    .interact()?;

    // Update enabled status for each extension
    for name in extension_status.iter().map(|(name, _)| name) {
        set_extension_enabled(
            &name_to_key(name),
            selected.iter().any(|s| s.as_str() == name),
        );
    }

    let config = Config::global();
    cliclack::outro(format!(
        "Capability and extension settings saved successfully to {}",
        config.path()
    ))?;
    Ok(())
}

fn prompt_extension_timeout() -> anyhow::Result<u64> {
    Ok(
        cliclack::input("Please set the timeout for this tool (in secs):")
            .placeholder(&biorouter::config::DEFAULT_EXTENSION_TIMEOUT.to_string())
            .validate(|input: &String| match input.parse::<u64>() {
                Ok(_) => Ok(()),
                Err(_) => Err("Please enter a valid timeout"),
            })
            .interact()?,
    )
}

fn prompt_extension_description() -> anyhow::Result<String> {
    Ok(cliclack::input("Enter a description for this extension:")
        .placeholder("Description")
        .validate(|input: &String| {
            if input.trim().is_empty() {
                Err("Please enter a valid description")
            } else {
                Ok(())
            }
        })
        .interact()?)
}

fn prompt_extension_name(placeholder: &str) -> anyhow::Result<String> {
    let extensions = get_all_extension_names();
    Ok(
        cliclack::input("What would you like to call this extension?")
            .placeholder(placeholder)
            .validate(move |input: &String| {
                if input.is_empty() {
                    Err("Please enter a name")
                } else if extensions.contains(input) {
                    Err("An extension with this name already exists")
                } else {
                    Ok(())
                }
            })
            .interact()?,
    )
}

fn collect_env_vars() -> anyhow::Result<(HashMap<String, String>, Vec<String>)> {
    let envs = HashMap::new();
    let mut env_keys = Vec::new();
    let config = Config::global();

    if !cliclack::confirm("Would you like to add environment variables?").interact()? {
        return Ok((envs, env_keys));
    }

    loop {
        let key: String = cliclack::input("Environment variable name:")
            .placeholder("API_KEY")
            .interact()?;

        let value: String = cliclack::password("Environment variable value:")
            .mask('▪')
            .interact()?;

        if !try_store_secret(config, &key, value)? {
            return Err(anyhow::anyhow!("Failed to store secret"));
        }
        env_keys.push(key);

        if !cliclack::confirm("Add another environment variable?").interact()? {
            break;
        }
    }

    Ok((envs, env_keys))
}

fn collect_headers() -> anyhow::Result<HashMap<String, String>> {
    let mut headers = HashMap::new();

    if !cliclack::confirm("Would you like to add custom headers?").interact()? {
        return Ok(headers);
    }

    loop {
        let key: String = cliclack::input("Header name:")
            .placeholder("Authorization")
            .interact()?;

        let value: String = cliclack::input("Header value:")
            .placeholder("Bearer token123")
            .interact()?;

        headers.insert(key, value);

        if !cliclack::confirm("Add another header?").interact()? {
            break;
        }
    }

    Ok(headers)
}

fn configure_builtin_extension() -> anyhow::Result<()> {
    let extensions = vec![
        (
            "autovisualiser",
            "Auto Visualiser",
            "Interactive charts, diagrams, networks, maps and scientific plots, shown in the side panel.",
        ),
        (
            "computercontroller",
            "Biorouter Copilot",
            "View and control the backend desktop after you approve each task.",
        ),
        (
            "webdocuments",
            "Web & Documents",
            "Read web pages and process PDF, Word, and spreadsheet documents.",
        ),
        (
            "developer",
            "Developer Tools",
            "Read, write and run code, and run shell commands.",
        ),
        (
            "memory",
            "Memory",
            "Teach Biorouter your preferences so it remembers them as you go.",
        ),
        (
            "agent_drafter",
            "Agent Drafter",
            "Build interactive artifacts, static pages or apps with an embedded Biorouter agent, and export them as standalone projects.",
        ),
    ];

    let mut select = cliclack::select("Which built-in extension would you like to enable?");
    for (id, name, desc) in &extensions {
        select = select.item(id, name, desc);
    }
    let extension = select.interact()?.to_string();
    let timeout = prompt_extension_timeout()?;

    let (display_name, description) = extensions
        .iter()
        .find(|(id, _, _)| id == &extension)
        .map(|(_, name, desc)| (name.to_string(), desc.to_string()))
        .unwrap_or_else(|| (extension.clone(), extension.clone()));

    set_extension(ExtensionEntry {
        enabled: true,
        config: ExtensionConfig::Builtin {
            name: extension.clone(),
            display_name: Some(display_name),
            timeout: Some(timeout),
            bundled: Some(true),
            description,
            available_tools: Vec::new(),
        },
    });

    cliclack::outro(format!("Enabled {} extension", style(extension).green()))?;
    Ok(())
}

fn configure_stdio_extension() -> anyhow::Result<()> {
    let name = prompt_extension_name("my-extension")?;

    let command_str: String = cliclack::input("What command should be run?")
        .placeholder("npx -y @example/my-server")
        .validate(|input: &String| {
            if input.is_empty() {
                Err("Please enter a command")
            } else {
                Ok(())
            }
        })
        .interact()?;

    let timeout = prompt_extension_timeout()?;

    let mut parts = command_str.split_whitespace();
    let cmd = parts.next().unwrap_or("").to_string();
    let args: Vec<String> = parts.map(String::from).collect();

    let description = prompt_extension_description()?;
    let (envs, env_keys) = collect_env_vars()?;

    set_extension(ExtensionEntry {
        enabled: true,
        config: ExtensionConfig::Stdio {
            name: name.clone(),
            cmd,
            args,
            envs: Envs::new(envs),
            env_keys,
            description,
            timeout: Some(timeout),
            bundled: None,
            available_tools: Vec::new(),
        },
    });

    cliclack::outro(format!("Added {} extension", style(name).green()))?;
    Ok(())
}

fn configure_streamable_http_extension() -> anyhow::Result<()> {
    let name = prompt_extension_name("my-remote-extension")?;

    let uri: String = cliclack::input("What is the Streaming HTTP endpoint URI?")
        .placeholder("http://localhost:8000/messages")
        .validate(|input: &String| {
            if input.is_empty() {
                Err("Please enter a URI")
            } else if !(input.starts_with("http://") || input.starts_with("https://")) {
                Err("URI should start with http:// or https://")
            } else {
                Ok(())
            }
        })
        .interact()?;

    let timeout = prompt_extension_timeout()?;
    let description = prompt_extension_description()?;
    let headers = collect_headers()?;

    // Original behavior: no env var collection for Streamable HTTP
    let envs = HashMap::new();
    let env_keys = Vec::new();

    set_extension(ExtensionEntry {
        enabled: true,
        config: ExtensionConfig::StreamableHttp {
            name: name.clone(),
            uri,
            envs: Envs::new(envs),
            env_keys,
            headers,
            description,
            timeout: Some(timeout),
            bundled: None,
            available_tools: Vec::new(),
        },
    });

    cliclack::outro(format!("Added {} extension", style(name).green()))?;
    Ok(())
}

pub fn configure_extensions_dialog() -> anyhow::Result<()> {
    let extension_type = cliclack::select("What type of extension would you like to add?")
        .item(
            "built-in",
            "Built-in Extension",
            "Use an extension that comes with biorouter",
        )
        .item(
            "stdio",
            "Command-line Extension",
            "Run a local command or script",
        )
        .item(
            "streamable_http",
            "Remote Extension (Streamable HTTP)",
            "Connect to a remote extension via MCP Streamable HTTP",
        )
        .interact()?;

    match extension_type {
        "built-in" => configure_builtin_extension()?,
        "stdio" => configure_stdio_extension()?,
        "streamable_http" => configure_streamable_http_extension()?,
        _ => unreachable!(),
    };

    print_config_file_saved()?;
    Ok(())
}

pub fn remove_extension_dialog() -> anyhow::Result<()> {
    for warning in biorouter::config::get_warnings() {
        eprintln!("{}", style(format!("Warning: {}", warning)).yellow());
    }

    let extensions = get_all_extensions();

    // Create a list of extension names and their enabled status
    let mut extension_status: Vec<(String, bool)> = extensions
        .iter()
        .map(|entry| (entry.config.name().to_string(), entry.enabled))
        .collect();

    // Sort extensions alphabetically by name
    extension_status.sort_by(|a, b| a.0.cmp(&b.0));

    if extensions.is_empty() {
        cliclack::outro(
            "No extensions configured yet. Run configure and add some extensions first.",
        )?;
        return Ok(());
    }

    // Check if all extensions are enabled
    if extension_status.iter().all(|(_, enabled)| *enabled) {
        cliclack::outro(
            "All extensions are currently enabled. You must first disable extensions before removing them.",
        )?;
        return Ok(());
    }

    // Filter out only disabled extensions
    let disabled_extensions: Vec<_> = extensions
        .iter()
        .filter(|entry| !entry.enabled)
        .map(|entry| (entry.config.name().to_string(), entry.enabled))
        .collect();

    let selected = cliclack::multiselect("Select extensions to remove (note: you can only remove disabled extensions - use \"space\" to toggle and \"enter\" to submit)")
        .required(false)
        .items(
            &disabled_extensions
                .iter()
                .filter(|(_, enabled)| !enabled)
                .map(|(name, _)| (name, name.as_str(), MULTISELECT_VISIBILITY_HINT))
                .collect::<Vec<_>>(),
        )
        .interact()?;

    for name in selected {
        remove_extension(&name_to_key(name));
        PermissionManager::instance().remove_extension(&name_to_key(name));
        cliclack::outro(format!("Removed {} extension", style(name).green()))?;
    }

    print_config_file_saved()?;

    Ok(())
}

pub async fn configure_settings_dialog() -> anyhow::Result<()> {
    let setting_type = cliclack::select("What setting would you like to configure?")
        .item(
            "biorouter_mode",
            "Biorouter mode",
            "Choose how much Biorouter does without asking",
        )
        .item(
            "tool_permission",
            "Tool Permission",
            "Set permission for individual tool of enabled extensions",
        )
        .item(
            "tool_output",
            "Tool Output",
            "Show more or less tool output",
        )
        .item(
            "max_turns",
            "Max Turns",
            "Set maximum number of turns without user input",
        )
        .item(
            "default_worker",
            "Default Worker Model",
            "The model subagents use unless the agent picks another",
        )
        .item(
            "keyring",
            "Secret Storage",
            "Configure how secrets are stored (keyring vs file)",
        )
        .item(
            "experiment",
            "Toggle Experiment",
            "Enable or disable an experiment feature",
        )
        .item(
            "workflow",
            "Workflow GitHub repository",
            "Where Biorouter looks for a workflow it does not find on this computer",
        )
        .interact()?;

    let mut should_print_config_path = true;

    match setting_type {
        "biorouter_mode" => {
            configure_biorouter_mode_dialog()?;
        }
        "tool_permission" => {
            configure_tool_permissions_dialog().await.and(Ok(()))?;
            // No need to print config file path since it's already handled.
            should_print_config_path = false;
        }
        "tool_output" => {
            configure_tool_output_dialog()?;
        }
        "max_turns" => {
            configure_max_turns_dialog()?;
        }
        "default_worker" => {
            configure_default_worker_dialog().await?;
        }
        "keyring" => {
            configure_keyring_dialog()?;
        }
        "experiment" => {
            toggle_experiments_dialog()?;
        }
        "workflow" => {
            configure_workflow_dialog()?;
        }
        _ => unreachable!(),
    };

    if should_print_config_path {
        print_config_file_saved()?;
    }

    Ok(())
}

/// What the dialog says about the current default worker model.
fn default_worker_status(worker: Option<&DefaultWorker>) -> String {
    match worker {
        Some(worker) => format!(
            "Subagents use {} unless the agent picks another model.",
            worker.label()
        ),
        None => "Subagents use the chat's model unless the agent picks another. Choose a \
                 lighter model to save tokens."
            .to_string(),
    }
}

/// Configure the default worker model: the model subagents use unless the
/// agent names another. It replaces lead/worker mode (owner, 2026-11-11) and
/// mirrors Settings > Models > Default worker model in the desktop app.
pub async fn configure_default_worker_dialog() -> anyhow::Result<()> {
    let config = Config::global();
    let current = configured_default_worker_in(config);
    let _ = cliclack::log::info(default_worker_status(current.as_ref()));

    let action = cliclack::select("What would you like to do?")
        .item(
            "choose",
            if current.is_some() {
                "Choose another default worker model"
            } else {
                "Choose a default worker model"
            },
            "",
        )
        .item(
            "same",
            "Same as the chat's model",
            if current.is_some() { "" } else { "(current)" },
        )
        .interact()?;

    if action == "same" {
        // Both keys in one write, so a reader never sees one half left behind.
        config.edit_values(|values| {
            values.shift_remove(WORKER_PROVIDER_KEY);
            values.shift_remove(WORKER_MODEL_KEY);
        })?;
        let _ = cliclack::log::success(default_worker_status(None));
        return Ok(());
    }

    let mut available_providers = chat_providers().await;
    available_providers.sort_by(|a, b| a.0.display_name.cmp(&b.0.display_name));
    let provider_items: Vec<(&String, &str, &str)> = available_providers
        .iter()
        .map(|(p, _)| (&p.name, p.display_name.as_str(), p.description.as_str()))
        .collect();
    let default_provider: String = current
        .as_ref()
        .map(|worker| worker.provider.clone())
        .or_else(|| config.get_biorouter_provider().ok())
        .unwrap_or_default();
    let provider_name = cliclack::select("Which provider should subagents use?")
        .initial_value(&default_provider)
        .items(&provider_items)
        .interact()?;
    let (provider_meta, _) = available_providers
        .iter()
        .find(|(p, _)| &p.name == provider_name)
        .expect("Selected provider must exist in metadata");

    let spin = spinner();
    spin.start("Fetching models...");
    let models_res = {
        let temp_model_config = ModelConfig::new(&provider_meta.default_model)?;
        let temp_provider = create(provider_name, temp_model_config).await?;
        retry_operation(&RetryConfig::default(), || async {
            temp_provider.fetch_recommended_models().await
        })
        .await
    };
    spin.stop(style("Model fetch complete").green());
    let current_model = current
        .as_ref()
        .filter(|worker| &worker.provider == provider_name)
        .map(|worker| worker.model.clone());
    let model: String = match models_res {
        Ok(Some(models)) if !models.is_empty() || provider_meta.name == "github_copilot" => {
            select_model_from_list(&models, provider_meta, current_model.as_deref())?
        }
        _ => {
            let default_model = current_model
                .clone()
                .unwrap_or_else(|| provider_meta.default_model.clone());
            prompt_for_unlisted_provider_model(
                provider_meta,
                current_model.as_deref(),
                "Enter the model name:",
                &default_model,
            )?
        }
    };

    // Both halves in one write, so a subagent never starts on a provider
    // paired with another provider's model.
    config.set_params(&[
        (WORKER_MODEL_KEY, model.as_str()),
        (WORKER_PROVIDER_KEY, provider_name.as_str()),
    ])?;
    let _ = cliclack::log::success(format!(
        "Subagents now use {} ({}) unless the agent picks another model.",
        model, provider_meta.display_name
    ));
    Ok(())
}

pub fn configure_biorouter_mode_dialog() -> anyhow::Result<()> {
    let config = Config::global();

    if std::env::var("BIOROUTER_MODE").is_ok() {
        let _ = cliclack::log::info("Notice: BIOROUTER_MODE environment variable is set and will override the configuration here.");
    }

    let mut select = cliclack::select("Which mode should Biorouter use?");
    for mode in MODES {
        let (label, description) = mode_choice(mode);
        select = select.item(mode, label, description);
    }
    let mode = select.interact()?;

    config.set_biorouter_mode(mode)?;
    cliclack::outro(format!("Mode set to {}.", mode_choice(mode).0))?;
    Ok(())
}

/// The modes `configure` offers, in the desktop picker's order.
const MODES: [BioRouterMode; 4] = [
    BioRouterMode::Auto,
    BioRouterMode::Approve,
    BioRouterMode::SmartApprove,
    BioRouterMode::Chat,
];

/// A mode's name and description, exactly as the desktop's approval mode menu
/// (`approvalsCopy.modes` in `ui/desktop/src/components/settings/chat/copy.ts`)
/// says them. `configure` used to say "Auto/Approve/Smart Approve/Chat Mode"
/// while the app said "Autonomous/Manual/Smart/Chat only", so a person meeting a
/// mode in one surface could not find it under the same name in the other. The
/// desktop's words win: when its copy changes, the test below fails until these
/// follow.
fn mode_choice(mode: BioRouterMode) -> (&'static str, &'static str) {
    match mode {
        BioRouterMode::Auto => ("Autonomous", "Uses tools and edits files without asking"),
        BioRouterMode::Approve => ("Manual", "Asks before using tools or changing files"),
        BioRouterMode::SmartApprove => ("Smart", "Asks only when an action looks risky"),
        BioRouterMode::Chat => ("Chat only", "Talks to the model with no tools"),
    }
}

pub fn configure_tool_output_dialog() -> anyhow::Result<()> {
    let config = Config::global();

    if std::env::var("BIOROUTER_CLI_MIN_PRIORITY").is_ok() {
        let _ = cliclack::log::info("Notice: BIOROUTER_CLI_MIN_PRIORITY environment variable is set and will override the configuration here.");
    }
    let tool_log_level = cliclack::select("Which tool output would you like to show?")
        .item("high", "High Importance", "")
        .item("medium", "Medium Importance", "Ex. results of file-writes")
        .item("all", "All (default)", "Ex. shell command output")
        .interact()?;

    match tool_log_level {
        "high" => {
            config.set_param("BIOROUTER_CLI_MIN_PRIORITY", 0.8)?;
            cliclack::outro("Showing tool output of high importance only.")?;
        }
        "medium" => {
            config.set_param("BIOROUTER_CLI_MIN_PRIORITY", 0.2)?;
            cliclack::outro("Showing tool output of medium importance.")?;
        }
        "all" => {
            config.set_param("BIOROUTER_CLI_MIN_PRIORITY", 0.0)?;
            cliclack::outro("Showing all tool output.")?;
        }
        _ => unreachable!(),
    };

    Ok(())
}

pub fn configure_keyring_dialog() -> anyhow::Result<()> {
    let config = Config::global();

    if std::env::var("BIOROUTER_DISABLE_KEYRING").is_ok() {
        let _ = cliclack::log::info("Notice: BIOROUTER_DISABLE_KEYRING environment variable is set and will override the configuration here.");
    }

    let currently_disabled = config
        .get_param::<String>("BIOROUTER_DISABLE_KEYRING")
        .is_ok();

    let current_status = if currently_disabled {
        "Disabled (using file-based storage)"
    } else {
        "Enabled (using system keyring)"
    };

    let _ = cliclack::log::info(format!("Current secret storage: {}", current_status));
    let _ = cliclack::log::warning("Note: Disabling the keyring stores secrets in a plain text file (~/.config/biorouter/secrets.yaml)");

    let storage_option = cliclack::select("How would you like to store secrets?")
        .item(
            "keyring",
            "System Keyring (recommended)",
            "Use secure system keyring for storing API keys and secrets",
        )
        .item(
            "file",
            "File-based Storage",
            "Store secrets in a local file (useful when keyring access is restricted)",
        )
        .interact()?;

    match storage_option {
        "keyring" => {
            // Set to empty string to enable keyring (absence or empty = enabled)
            config.set_param("BIOROUTER_DISABLE_KEYRING", Value::String("".to_string()))?;
            cliclack::outro("Secret storage set to system keyring (secure)")?;
            let _ = cliclack::log::info(
                "You may need to restart biorouter for this change to take effect",
            );
        }
        "file" => {
            // Set the disable flag to use file storage
            config.set_param(
                "BIOROUTER_DISABLE_KEYRING",
                Value::String("true".to_string()),
            )?;
            cliclack::outro(
                "Secret storage set to file (~/.config/biorouter/secrets.yaml). Keep this file secure!",
            )?;
            let _ = cliclack::log::info(
                "You may need to restart biorouter for this change to take effect",
            );
        }
        _ => unreachable!(),
    };

    Ok(())
}

/// Configure experiment features that can be used with biorouter
/// Dialog for toggling which experiments are enabled/disabled
pub fn toggle_experiments_dialog() -> anyhow::Result<()> {
    let experiments = ExperimentManager::get_all()?;

    if experiments.is_empty() {
        cliclack::outro("No experiments supported yet.")?;
        return Ok(());
    }

    // Get currently enabled experiments for the selection
    let enabled_experiments: Vec<&String> = experiments
        .iter()
        .filter(|(_, enabled)| *enabled)
        .map(|(name, _)| name)
        .collect();

    // Let user toggle experiments
    let selected = cliclack::multiselect(
        "enable experiments: (use \"space\" to toggle and \"enter\" to submit)",
    )
    .required(false)
    .items(
        &experiments
            .iter()
            .map(|(name, _)| (name, name.as_str(), MULTISELECT_VISIBILITY_HINT))
            .collect::<Vec<_>>(),
    )
    .initial_values(enabled_experiments)
    .interact()?;

    // Update enabled status for each experiments
    for name in experiments.iter().map(|(name, _)| name) {
        ExperimentManager::set_enabled(name, selected.iter().any(|&s| s.as_str() == name))?;
    }

    cliclack::outro("Experiments settings updated successfully")?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
pub async fn configure_tool_permissions_dialog() -> anyhow::Result<()> {
    let mut extensions: Vec<String> = get_enabled_extensions()
        .into_iter()
        .map(|ext| ext.name().clone())
        .collect();
    extensions.push("platform".to_string());

    extensions.sort();

    let selected_extension_name = cliclack::select("Choose an extension to configure tools")
        .items(
            &extensions
                .iter()
                .map(|ext| (ext.clone(), ext.clone(), ""))
                .collect::<Vec<_>>(),
        )
        .interact()?;

    let config = Config::global();

    let provider_name: String = config
        .get_biorouter_provider()
        .expect("No provider configured. Please set model provider first");

    let model: String = config
        .get_biorouter_model()
        .expect("No model configured. Please set model first");
    let model_config = ModelConfig::new(&model)?;

    let agent = Agent::new();
    let new_provider = create(&provider_name, model_config).await?;

    let session = agent
        .config
        .session_manager
        .create_session(
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            "Tool Permission Configuration".to_string(),
            SessionType::Hidden,
        )
        .await?;

    agent.update_provider(new_provider, &session.id).await?;
    if let Some(config) = get_extension_by_name(&selected_extension_name) {
        agent
            .add_extension(config.clone())
            .await
            .unwrap_or_else(|_| {
                println!(
                    "{} Failed to check extension: {}",
                    style("Error").red().italic(),
                    config.name()
                );
            });
    } else {
        println!(
            "{} Configuration not found for extension: {}",
            style("Warning").yellow().italic(),
            selected_extension_name
        );
        return Ok(());
    }

    let permission_manager = PermissionManager::instance();
    // Issue #56 Task 16: the permission editor's list, NOT the model's. The
    // provider bound just above is whatever the user configured, so under Gate E
    // this selector would be handed zero items for a private extension and the
    // user could never set a tool permission on the extension they installed.
    // Do not swap this for `list_tools`.
    let selected_tools = agent
        .list_tools_for_permission_settings(&session.id, Some(selected_extension_name.clone()))
        .await
        .into_iter()
        .map(|tool| {
            ToolInfo::new(
                &tool.name,
                tool.description
                    .as_ref()
                    .map(|d| d.as_ref())
                    .unwrap_or_default(),
                get_parameter_names(&tool),
                permission_manager.get_user_permission(&tool.name),
            )
        })
        .collect::<Vec<ToolInfo>>();

    let tool_name = cliclack::select("Choose a tool to update permission")
        .items(
            &selected_tools
                .iter()
                .map(|tool| {
                    let first_description = tool
                        .description
                        .split('.')
                        .next()
                        .unwrap_or("No description available")
                        .trim();
                    (tool.name.clone(), tool.name.clone(), first_description)
                })
                .collect::<Vec<_>>(),
        )
        .interact()?;

    // Find the selected tool
    let tool = selected_tools
        .iter()
        .find(|tool| tool.name == tool_name)
        .unwrap();

    // Display tool description and current permission level
    let current_permission = match tool.permission {
        Some(PermissionLevel::AlwaysAllow) => "Always Allow",
        Some(PermissionLevel::AskBefore) => "Ask Before",
        Some(PermissionLevel::NeverAllow) => "Never Allow",
        None => "Not Set",
    };

    // Allow user to set the permission level
    let permission = cliclack::select(format!(
        "Set permission level for tool {}, current permission level: {}",
        tool.name, current_permission
    ))
    .item(
        "always_allow",
        "Always Allow",
        "Allow this tool to execute without asking",
    )
    .item(
        "ask_before",
        "Ask Before",
        "Prompt before executing this tool",
    )
    .item(
        "never_allow",
        "Never Allow",
        "Prevent this tool from executing",
    )
    .interact()?;

    let permission_label = match permission {
        "always_allow" => "Always Allow",
        "ask_before" => "Ask Before",
        "never_allow" => "Never Allow",
        _ => unreachable!(),
    };

    // Update the permission level in the configuration
    let new_permission = match permission {
        "always_allow" => PermissionLevel::AlwaysAllow,
        "ask_before" => PermissionLevel::AskBefore,
        "never_allow" => PermissionLevel::NeverAllow,
        _ => unreachable!(),
    };

    permission_manager.update_user_permission(&tool.name, new_permission);

    cliclack::outro(format!(
        "Updated permission level for tool {} to {}.",
        tool.name, permission_label
    ))?;

    cliclack::outro(format!(
        "Changes saved to {}",
        permission_manager.get_config_path().display()
    ))?;

    Ok(())
}

fn configure_workflow_dialog() -> anyhow::Result<()> {
    let key_name = BIOROUTER_WORKFLOW_GITHUB_REPO_CONFIG_KEY;
    let config = Config::global();
    let default_workflow_repo = std::env::var(key_name)
        .ok()
        .or_else(|| config.get_param(key_name).unwrap_or(None));
    let mut workflow_repo_input = cliclack::input(
        "Enter your biorouter workflow Github repo (owner/repo): eg: my_org/biorouter-workflows",
    )
    .required(false);
    if let Some(workflow_repo) = default_workflow_repo {
        workflow_repo_input = workflow_repo_input.default_input(&workflow_repo);
    }
    let input_value: String = workflow_repo_input.interact()?;
    if input_value.clone().trim().is_empty() {
        config.delete(key_name)?;
    } else {
        config.set_param(key_name, &input_value)?;
    }
    Ok(())
}

pub fn configure_max_turns_dialog() -> anyhow::Result<()> {
    let config = Config::global();

    let current_max_turns: u32 = config
        .get_param("BIOROUTER_MAX_TURNS")
        .unwrap_or(biorouter::agents::DEFAULT_MAX_TURNS);

    let max_turns_input: String =
        cliclack::input("Set maximum agent turns without user input (0 = unlimited):")
            .placeholder(&current_max_turns.to_string())
            .default_input(&current_max_turns.to_string())
            .validate(|input: &String| parse_max_turns(input).map(|_| ()))
            .interact()?;

    let max_turns = parse_max_turns(&max_turns_input).map_err(anyhow::Error::msg)?;
    config.set_param("BIOROUTER_MAX_TURNS", max_turns)?;

    let message = if max_turns == 0 {
        "Max turns set to Unlimited: useful work can continue until completion or Stop.".to_string()
    } else {
        format!(
            "Max turns set to {max_turns}: Biorouter asks for your input after {max_turns} actions in a row."
        )
    };
    cliclack::outro(message)?;

    Ok(())
}

/// Zero selects unlimited turns; an empty entry never silently removes a limit.
fn parse_max_turns(input: &str) -> Result<u32, &'static str> {
    input.trim().parse::<u32>().map_err(|_| MAX_TURNS_REFUSAL)
}

/// The CLI accepts zero for Unlimited; Settings offers that as a separate choice.
const MAX_TURNS_REFUSAL: &str = "Enter a whole number of at least 0.";

/// Handle OpenRouter authentication
pub async fn handle_openrouter_auth() -> anyhow::Result<()> {
    use biorouter::config::{configure_openrouter, signup_openrouter::OpenRouterAuth};
    use biorouter::conversation::message::Message;
    use biorouter::providers::create;

    // Use the OpenRouter authentication flow
    let mut auth_flow = OpenRouterAuth::new()?;
    let api_key = auth_flow.complete_flow().await?;
    println!("\nAuthentication complete!");

    // Get config instance
    let config = Config::global();

    // Use the existing configure_openrouter function to set everything up
    println!("\nConfiguring OpenRouter...");
    configure_openrouter(config, api_key)?;

    println!("✓ OpenRouter configuration complete");
    println!("✓ Models configured successfully");

    // Test configuration - get the model that was configured
    println!("\nTesting configuration...");
    let configured_model: String = config.get_biorouter_model()?;
    let model_config = match biorouter::model::ModelConfig::new(&configured_model) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("⚠️  Invalid model configuration: {}", e);
            eprintln!("Your settings have been saved. Please check your model configuration.");
            return Ok(());
        }
    };

    match create("openrouter", model_config).await {
        Ok(provider) => {
            // Simple test request
            let test_result = provider
                .complete(
                    "You are biorouter, an AI assistant.",
                    &[Message::user().with_text("Say 'Configuration test successful!'")],
                    &[],
                )
                .await;

            match test_result {
                Ok(_) => {
                    println!("✓ Configuration test passed!");

                    // Enable the developer extension by default if not already enabled
                    let entries = get_all_extensions();
                    let has_developer = entries
                        .iter()
                        .any(|e| e.config.name() == "developer" && e.enabled);

                    if !has_developer {
                        set_extension(ExtensionEntry {
                            enabled: true,
                            config: ExtensionConfig::Builtin {
                                name: "developer".to_string(),
                                display_name: Some(
                                    biorouter::config::DEFAULT_DISPLAY_NAME.to_string(),
                                ),
                                timeout: Some(biorouter::config::DEFAULT_EXTENSION_TIMEOUT),
                                bundled: Some(true),
                                description: "Developer capability".to_string(),
                                available_tools: Vec::new(),
                            },
                        });
                        println!("✓ Developer capability enabled");
                    }

                    cliclack::outro("OpenRouter setup complete! You can now use biorouter.")?;
                }
                Err(e) => {
                    eprintln!("⚠️  Configuration test failed: {}", e);
                    eprintln!("Your settings have been saved, but there may be an issue with the connection.");
                }
            }
        }
        Err(e) => {
            eprintln!("⚠️  Failed to create provider for testing: {}", e);
            eprintln!("Your settings have been saved. Please check your configuration.");
        }
    }
    Ok(())
}

pub async fn handle_tetrate_auth() -> anyhow::Result<()> {
    let mut auth_flow = TetrateAuth::new()?;
    let api_key = auth_flow.complete_flow().await?;

    println!("\nAuthentication complete!");

    let config = Config::global();

    println!("\nConfiguring Tetrate Agent Router Service...");
    configure_tetrate(config, api_key)?;

    println!("✓ Tetrate Agent Router Service configuration complete");
    println!("✓ Models configured successfully");

    // Test configuration
    println!("\nTesting configuration...");
    let configured_model: String = config.get_biorouter_model()?;
    let model_config = match biorouter::model::ModelConfig::new(&configured_model) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("⚠️  Invalid model configuration: {}", e);
            eprintln!("Your settings have been saved. Please check your model configuration.");
            return Ok(());
        }
    };

    match create("tetrate", model_config).await {
        Ok(provider) => {
            let test_result = provider
                .complete(
                    "You are biorouter, an AI assistant.",
                    &[Message::user().with_text("Say 'Configuration test successful!'")],
                    &[],
                )
                .await;

            match test_result {
                Ok(_) => {
                    println!("✓ Configuration test passed!");

                    let entries = get_all_extensions();
                    let has_developer = entries
                        .iter()
                        .any(|e| e.config.name() == "developer" && e.enabled);

                    if !has_developer {
                        set_extension(ExtensionEntry {
                            enabled: true,
                            config: ExtensionConfig::Builtin {
                                name: "developer".to_string(),
                                display_name: Some(
                                    biorouter::config::DEFAULT_DISPLAY_NAME.to_string(),
                                ),
                                timeout: Some(biorouter::config::DEFAULT_EXTENSION_TIMEOUT),
                                bundled: Some(true),
                                description: "Developer capability".to_string(),
                                available_tools: Vec::new(),
                            },
                        });
                        println!("✓ Developer capability enabled");
                    }

                    cliclack::outro(
                        "Tetrate Agent Router Service setup complete! You can now use biorouter.",
                    )?;
                }
                Err(e) => {
                    eprintln!("⚠️  Configuration test failed: {}", e);
                    eprintln!("Your settings have been saved, but there may be an issue with the connection.");
                }
            }
        }
        Err(e) => {
            eprintln!("⚠️  Failed to create provider for testing: {}", e);
            eprintln!("Your settings have been saved. Please check your configuration.");
        }
    }

    Ok(())
}

/// Prompts the user to collect custom HTTP headers for a provider.
fn collect_custom_headers() -> anyhow::Result<Option<std::collections::HashMap<String, String>>> {
    let use_custom_headers = cliclack::confirm("Does this provider require custom headers?")
        .initial_value(false)
        .interact()?;

    if !use_custom_headers {
        return Ok(None);
    }

    let mut custom_headers = std::collections::HashMap::new();

    loop {
        let header_name: String = cliclack::input("Header name:")
            .placeholder("e.g., x-origin-client-id")
            .required(false)
            .interact()?;

        if header_name.is_empty() {
            break;
        }

        let header_value: String = cliclack::password(format!("Value for '{}':", header_name))
            .mask('▪')
            .interact()?;

        custom_headers.insert(header_name, header_value);

        let add_more = cliclack::confirm("Add another header?")
            .initial_value(false)
            .interact()?;

        if !add_more {
            break;
        }
    }

    if custom_headers.is_empty() {
        Ok(None)
    } else {
        Ok(Some(custom_headers))
    }
}

fn add_provider() -> anyhow::Result<()> {
    let provider_type = cliclack::select("What type of API is this?")
        .item(
            "openai_compatible",
            "OpenAI Compatible",
            "Uses OpenAI API format",
        )
        .item(
            "anthropic_compatible",
            "Anthropic Compatible",
            "Uses Anthropic API format",
        )
        .item(
            "ollama_compatible",
            "Ollama Compatible",
            "Uses Ollama API format",
        )
        .interact()?;

    let display_name: String = cliclack::input("What should we call this provider?")
        .placeholder("Your Provider Name")
        .validate(|input: &String| {
            if input.is_empty() {
                Err("Please enter a name")
            } else {
                Ok(())
            }
        })
        .interact()?;

    let api_url: String = cliclack::input("Provider API URL:")
        .placeholder("https://api.example.com/v1")
        .validate(|input: &String| {
            if !input.starts_with("http://") && !input.starts_with("https://") {
                Err("URL must start with either http:// or https://")
            } else {
                Ok(())
            }
        })
        .interact()?;

    let api_key: String = cliclack::password("API key:")
        .allow_empty()
        .mask('▪')
        .interact()?;

    let models_input: String = cliclack::input("Available models (separate with commas):")
        .placeholder("model-a, model-b, model-c")
        .validate(|input: &String| {
            if input.trim().is_empty() {
                Err("Please enter at least one model name")
            } else {
                Ok(())
            }
        })
        .interact()?;

    let models: Vec<String> = models_input
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let supports_streaming = cliclack::confirm("Does this provider support streaming responses?")
        .initial_value(true)
        .interact()?;

    // Ask about custom headers for OpenAI compatible providers
    let headers = if provider_type == "openai_compatible" {
        collect_custom_headers()?
    } else {
        None
    };

    create_custom_provider(
        provider_type,
        display_name.clone(),
        api_url,
        api_key,
        models,
        Some(supports_streaming),
        headers,
    )?;

    cliclack::outro(format!("Custom provider added: {}", display_name))?;
    Ok(())
}

fn remove_provider() -> anyhow::Result<()> {
    let custom_providers_dir = biorouter::config::declarative_providers::custom_providers_dir();
    let custom_providers = if custom_providers_dir.exists() {
        biorouter::config::declarative_providers::load_custom_providers(&custom_providers_dir)?
    } else {
        Vec::new()
    };

    if custom_providers.is_empty() {
        cliclack::outro("No custom providers added just yet.")?;
        return Ok(());
    }

    let provider_items: Vec<_> = custom_providers
        .iter()
        .map(|p| (p.name.as_str(), p.display_name.as_str(), "Custom provider"))
        .collect();

    let selected_id = cliclack::select("Which custom provider would you like to remove?")
        .items(&provider_items)
        .interact()?;

    remove_custom_provider(selected_id)?;
    cliclack::outro(format!("Removed custom provider: {}", selected_id))?;
    Ok(())
}

pub fn configure_custom_provider_dialog() -> anyhow::Result<()> {
    let action = cliclack::select("What would you like to do?")
        .item(
            "add",
            "Add A Custom Provider",
            "Add a new OpenAI/Anthropic/Ollama compatible Provider",
        )
        .item(
            "remove",
            "Remove Custom Provider",
            "Remove an existing custom provider",
        )
        .interact()?;

    match action {
        "add" => add_provider(),
        "remove" => remove_provider(),
        _ => unreachable!(),
    }?;

    print_config_file_saved()?;

    Ok(())
}

fn print_config_file_saved() -> anyhow::Result<()> {
    let config = Config::global();
    cliclack::outro(format!(
        "Configuration saved successfully to {}",
        config.path()
    ))?;
    Ok(())
}

/// Issue #56, DR-17 requirement 3: what a non-private model can reach, for the
/// terminal.
///
/// R10 makes the CLI a required surface, not an optional one — a user who never
/// opens the desktop app must still be told. Non-blocking by design: there is no
/// action to gate here, only a fact to convey, and a terminal prompt the user
/// has to dismiss to configure a provider is a prompt they learn to skip.
///
/// ⚠ Three properties, each with a Step 5 gate:
///   * the words come from `biorouter::privacy::disclosure`, never from a
///     literal here — one definition, four surfaces;
///   * the predicate is the provider's TIER, so a fourth private provider stops
///     triggering it with no edit in this file;
///   * it does **not** consult the master privacy switch. DR-15 turns off gates,
///     the ratchet and refusals; it does not turn off the truth, and with
///     enforcement off the exposure is larger, not smaller.
fn non_private_model_disclosure(
    provider_meta: &biorouter::providers::base::ProviderMetadata,
) -> Option<String> {
    use biorouter::privacy::disclosure;
    if !disclosure::required_for(provider_meta) {
        return None;
    }
    Some(format!(
        "{}\n{}",
        disclosure::title_for(&provider_meta.display_name),
        disclosure::COPY_SHORT
    ))
}

/// Print it, if this provider needs it. Separated from the predicate above so
/// the predicate is testable without a terminal.
fn print_non_private_model_disclosure(
    provider_meta: &biorouter::providers::base::ProviderMetadata,
) -> anyhow::Result<()> {
    if let Some(note) = non_private_model_disclosure(provider_meta) {
        cliclack::log::warning(note)?;
    }
    Ok(())
}

#[cfg(test)]
mod decision_provider_tests {
    use super::{chat_providers, default_worker_status};
    use biorouter::providers::providers;

    /// The two decision providers are registered, and neither is offered as a
    /// chat's provider or as a default worker model: a decision model cannot
    /// pass the chat check, and it cannot carry out a subagent's task.
    #[tokio::test]
    async fn configure_offers_no_decision_provider() {
        let registered: Vec<String> = providers().await.into_iter().map(|(m, _)| m.name).collect();
        for name in ["ollama_decision", "baseten_decision"] {
            assert!(registered.iter().any(|n| n == name), "{name} is registered");
        }
        let offered: Vec<String> = chat_providers()
            .await
            .into_iter()
            .map(|(m, _)| m.name)
            .collect();
        assert!(
            !offered.iter().any(|n| n.ends_with("_decision")),
            "{offered:?}"
        );
        assert!(offered.iter().any(|n| n == "ollama"), "{offered:?}");
        assert_eq!(offered.len(), registered.len() - 2);
    }

    #[test]
    fn the_dialog_says_what_subagents_use() {
        use biorouter::agents::default_worker::DefaultWorker;
        assert_eq!(
            default_worker_status(None),
            "Subagents use the chat's model unless the agent picks another. Choose a lighter \
             model to save tokens."
        );
        let worker = DefaultWorker {
            provider: "openai".into(),
            model: "gpt-4o-mini".into(),
        };
        assert_eq!(
            default_worker_status(Some(&worker)),
            "Subagents use gpt-4o-mini (openai) unless the agent picks another model."
        );
    }
}

#[cfg(test)]
mod privacy_disclosure_tests {
    use super::non_private_model_disclosure;
    use biorouter::privacy::disclosure;
    use biorouter::providers::providers;

    async fn meta(name: &str) -> biorouter::providers::base::ProviderMetadata {
        providers()
            .await
            .into_iter()
            .find(|(m, _)| m.name == name)
            .map(|(m, _)| m)
            .unwrap_or_else(|| panic!("no registry entry for `{name}`"))
    }

    #[tokio::test]
    async fn the_cli_tells_a_public_provider_and_stays_quiet_for_a_private_one() {
        let public = non_private_model_disclosure(&meta("openai").await)
            .expect("a public provider must be disclosed in the terminal too");
        // The served constants, not a fourth hand-written copy.
        assert!(public.contains(disclosure::COPY_SHORT), "{public}");
        assert!(
            public.contains("not hosted by your institution"),
            "{public}"
        );

        assert!(non_private_model_disclosure(&meta("llamacpp").await).is_none());
        assert!(non_private_model_disclosure(&meta("versa_azure").await).is_none());
    }
}

#[cfg(test)]
mod needs_terminal_tests {
    use super::{configure, CONFIGURE_NEEDS_A_TERMINAL};
    use crate::commands::needs_terminal::NeedsTerminal;
    use biorouter::config::Config;
    use std::ffi::OsString;

    fn entries(dir: &std::path::Path) -> Vec<OsString> {
        let mut names: Vec<OsString> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        names
    }

    /// QA-D F9: under a pipe, `configure` refuses with a sentence that names
    /// what to run instead — and the configuration is **byte-identical**
    /// afterwards, with nothing written beside it.
    ///
    /// Driven through `configure` itself, the function `handle_configure`
    /// calls, so moving the check below anything that touches the file fails
    /// here. The file carries a comment and odd spacing on purpose: a
    /// parse-and-re-serialise round trip would normalise both, so "identical
    /// bytes" cannot be satisfied by a write that happened to preserve the keys.
    #[tokio::test]
    async fn without_a_terminal_configure_refuses_and_leaves_the_config_byte_identical() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("config.yaml");
        let original: &[u8] = b"# hand-edited\nBIOROUTER_PROVIDER:   versa_azure\n\
                                BIOROUTER_MODEL: gpt-5.5-2026-04-24\n";
        std::fs::write(&config_path, original).unwrap();
        let config =
            Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml")).unwrap();

        let err = configure(&config, false).await.unwrap_err();
        let refusal = err
            .downcast_ref::<NeedsTerminal>()
            .expect("the refusal must be the typed one main maps to exit 2");
        assert_eq!(refusal.to_string(), CONFIGURE_NEEDS_A_TERMINAL);
        assert!(
            CONFIGURE_NEEDS_A_TERMINAL.contains("biorouter models set --provider"),
            "the sentence must name the non-interactive route: {CONFIGURE_NEEDS_A_TERMINAL}"
        );

        assert_eq!(std::fs::read(&config_path).unwrap(), original);
        assert_eq!(entries(dir.path()), vec![OsString::from("config.yaml")]);
    }

    /// The first-run branch (no config yet) refuses the same way and creates
    /// nothing — no config file, no secrets file.
    #[tokio::test]
    async fn without_a_terminal_first_run_configure_creates_nothing() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = Config::new_with_file_secrets(
            dir.path().join("config.yaml"),
            dir.path().join("secrets.yaml"),
        )
        .unwrap();

        let err = configure(&config, false).await.unwrap_err();
        assert!(err.downcast_ref::<NeedsTerminal>().is_some(), "{err:?}");
        assert!(
            entries(dir.path()).is_empty(),
            "a refused first run must leave no file behind: {:?}",
            entries(dir.path())
        );
    }
}

#[cfg(test)]
mod mode_name_tests {
    use super::{mode_choice, MODES};
    use std::path::Path;

    /// `configure` names and describes every mode exactly as the desktop's approval
    /// mode menu does, read from the copy module that menu renders so a rename on
    /// either side fails here.
    #[test]
    fn configure_uses_the_desktop_mode_names() {
        let picker = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../ui/desktop/src/components/settings/chat/copy.ts"),
        )
        .expect("the desktop approval mode copy");
        for mode in MODES {
            let (label, description) = mode_choice(mode);
            assert!(
                picker.contains(&format!("label: '{label}'")),
                "the desktop has no mode named {label:?}"
            );
            assert!(
                picker.contains(&format!("description: '{description}'")),
                "the desktop describes {label} differently from {description:?}"
            );
        }
        for retired in [
            "Auto Mode",
            "Approve Mode",
            "Smart Approve Mode",
            "Chat Mode",
        ] {
            assert!(
                MODES.iter().all(|mode| mode_choice(*mode).0 != retired),
                "{retired}"
            );
        }
    }
}

#[cfg(test)]
mod max_turns_tests {
    use super::parse_max_turns;

    /// The CLI and stored setting accept zero for Unlimited. Settings expresses
    /// Unlimited as a radio choice, so its finite input requires a positive value.
    /// That interaction is covered by `ConversationLimitsDropdown.test.tsx`.
    #[test]
    fn configure_accepts_unlimited_or_a_positive_limit() {
        for refused in ["", " ", "-5", "2.5", "1e3", "many", "4294967296"] {
            let refusal = parse_max_turns(refused).expect_err(refused);
            assert_eq!(
                refusal, "Enter a whole number of at least 0.",
                "{refused:?}"
            );
        }
        for (input, expected) in [
            ("0", 0),
            (" 0 ", 0),
            ("1", 1),
            (" 250 ", 250),
            ("4294967295", u32::MAX),
        ] {
            assert_eq!(parse_max_turns(input), Ok(expected), "{input:?}");
        }
    }
}

#[cfg(test)]
mod previous_provider_settings_tests {
    use super::PreviousProviderSettings;
    use biorouter::config::Config;
    use biorouter::providers::base::ConfigKey;

    /// W2-PRV-2: a key the provider refused had already replaced the working
    /// one. Whatever `configure` wrote is put back to what was saved before,
    /// and a setting that did not exist before is removed again.
    #[test]
    fn a_refused_setup_leaves_the_saved_settings_as_they_were() {
        let dir = tempfile::TempDir::new().unwrap();
        let config: &'static Config = Box::leak(Box::new(
            Config::new_with_file_secrets(
                dir.path().join("config.yaml"),
                dir.path().join("secrets.yaml"),
            )
            .unwrap(),
        ));
        config
            .set_secret("W2PRV2_PROBE_API_KEY", &"working-key")
            .unwrap();
        config
            .set_param("W2PRV2_PROBE_HOST", "https://api.example.test")
            .unwrap();
        let keys = vec![
            ConfigKey::new("W2PRV2_PROBE_API_KEY", true, true, None),
            ConfigKey::new("W2PRV2_PROBE_HOST", true, false, None),
            ConfigKey::new("W2PRV2_PROBE_ORG", false, false, None),
        ];

        let mut previous = PreviousProviderSettings::capture(config, &keys);
        config
            .set_secret("W2PRV2_PROBE_API_KEY", &"typo-key")
            .unwrap();
        config
            .set_param("W2PRV2_PROBE_HOST", "https://elsewhere.test")
            .unwrap();
        config.set_param("W2PRV2_PROBE_ORG", "new-org").unwrap();

        assert!(previous.restore().unwrap());
        assert_eq!(
            config.get_secret::<String>("W2PRV2_PROBE_API_KEY").unwrap(),
            "working-key"
        );
        assert_eq!(
            config.get_param::<String>("W2PRV2_PROBE_HOST").unwrap(),
            "https://api.example.test"
        );
        assert!(config.get_param::<String>("W2PRV2_PROBE_ORG").is_err());

        // Nothing changed since: nothing to put back.
        assert!(!previous.restore().unwrap());
    }

    /// An interrupt or error part-way through the dialog drops the capture
    /// without `keep`, and that puts the settings back too.
    #[test]
    fn leaving_the_dialog_early_puts_the_settings_back() {
        let dir = tempfile::TempDir::new().unwrap();
        let config: &'static Config = Box::leak(Box::new(
            Config::new_with_file_secrets(
                dir.path().join("config.yaml"),
                dir.path().join("secrets.yaml"),
            )
            .unwrap(),
        ));
        config
            .set_secret("W2PRV2_DROP_API_KEY", &"working-key")
            .unwrap();
        let keys = vec![ConfigKey::new("W2PRV2_DROP_API_KEY", true, true, None)];
        {
            let _previous = PreviousProviderSettings::capture(config, &keys);
            config
                .set_secret("W2PRV2_DROP_API_KEY", &"half-typed")
                .unwrap();
        }
        assert_eq!(
            config.get_secret::<String>("W2PRV2_DROP_API_KEY").unwrap(),
            "working-key"
        );
        {
            let mut previous = PreviousProviderSettings::capture(config, &keys);
            config
                .set_secret("W2PRV2_DROP_API_KEY", &"accepted")
                .unwrap();
            previous.keep();
        }
        assert_eq!(
            config.get_secret::<String>("W2PRV2_DROP_API_KEY").unwrap(),
            "accepted"
        );
    }
}

#[cfg(test)]
mod typed_settings_tests {
    use super::{check_then_save, secret_home, typed_overrides, TypedSetting, MAX_TURNS_REFUSAL};
    use biorouter::config::Config;

    fn leaked_config(dir: &tempfile::TempDir) -> &'static Config {
        Box::leak(Box::new(
            Config::new_with_file_secrets(
                dir.path().join("config.yaml"),
                dir.path().join("secrets.yaml"),
            )
            .unwrap(),
        ))
    }

    fn typed() -> Vec<TypedSetting> {
        vec![
            TypedSetting {
                name: "T3SH5_PROBE_API_KEY".to_string(),
                secret: true,
                value: "1234567890".to_string(),
            },
            TypedSetting {
                name: "T3SH5_PROBE_HOST".to_string(),
                secret: false,
                value: "https://typed.example.test".to_string(),
            },
        ]
    }

    #[test]
    fn typed_values_become_overrides_the_way_the_daemon_passes_them() {
        let overrides = typed_overrides(&typed());
        // A secret is a JSON string literal, so an all-digit key stays a string.
        assert_eq!(overrides["T3SH5_PROBE_API_KEY"], "\"1234567890\"");
        assert_eq!(overrides["T3SH5_PROBE_HOST"], "https://typed.example.test");
    }

    /// T3-SH-5: a key was written as it was typed, so a session killed before
    /// the check finished left it saved over the working one.
    #[tokio::test]
    async fn nothing_typed_is_written_unless_the_check_passes() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = leaked_config(&dir);
        config
            .set_secret("T3SH5_PROBE_API_KEY", &"working-key")
            .unwrap();

        // The check sees what was typed…
        let refused = check_then_save::<(), _>(config, &typed(), async {
            let host: String = Config::global().get_param("T3SH5_PROBE_HOST")?;
            let key: String = Config::global().get_secret("T3SH5_PROBE_API_KEY")?;
            assert_eq!(host, "https://typed.example.test");
            assert_eq!(key, "1234567890");
            anyhow::bail!("401 Unauthorized: Invalid client id or secret")
        })
        .await;
        // …and a refusal writes none of it.
        assert!(refused.is_err());
        assert_eq!(
            config.get_secret::<String>("T3SH5_PROBE_API_KEY").unwrap(),
            "working-key"
        );
        assert!(config.get_param::<String>("T3SH5_PROBE_HOST").is_err());

        let accepted = check_then_save::<(), _>(config, &typed(), async { Ok(()) }).await;
        assert!(matches!(accepted, Ok(Some(()))), "{accepted:?}");
        assert_eq!(
            config.get_secret::<String>("T3SH5_PROBE_API_KEY").unwrap(),
            "1234567890"
        );
        assert_eq!(
            config.get_param::<String>("T3SH5_PROBE_HOST").unwrap(),
            "https://typed.example.test"
        );
    }

    /// The dialog itself: no value is stored before `check_then_save`. Read
    /// at the source, because the dialog is a terminal conversation.
    #[test]
    fn the_provider_dialog_stores_nothing_before_the_check() {
        let source = include_str!("configure.rs");
        let (_, body) = source
            .split_once("pub async fn configure_provider_dialog()")
            .expect("the dialog");
        let (body, _) = body.split_once("\n}\n").expect("its end");
        let body: String = body
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let checked_at = body
            .find("check_then_save(")
            .expect("the dialog checks first");
        for write in [
            "try_store_secret(",
            "store_secret_reporting(",
            "set_param(",
            "set_secret(",
            "save_typed_settings(",
        ] {
            if let Some(at) = body.find(write) {
                assert!(
                    at > checked_at,
                    "the dialog writes with {write} before its check"
                );
            }
        }
    }

    /// T3-SH-6: the confirmation names where a secret really went.
    #[test]
    fn a_secret_is_said_to_be_where_it_went() {
        assert_eq!(secret_home(false, false), "the system keychain");
        for (file_before, fell_back) in [(true, false), (false, true), (true, true)] {
            let home = secret_home(file_before, fell_back);
            assert!(home.starts_with("the private file "), "{home}");
            assert!(home.ends_with("secrets.yaml"), "{home}");
        }
        assert!(MAX_TURNS_REFUSAL.ends_with('.'));
    }
}

#[cfg(test)]
mod llamacpp_picker_tests {
    use super::llamacpp_source_label;
    use biorouter::providers::llamacpp::MODEL_CATALOG;

    /// The picker never sends a user to pull a bare manifest-list tag, which
    /// is an MLX build on Apple Silicon that llama-server cannot reuse.
    #[test]
    fn the_picker_names_the_ollama_gguf_tag_llama_server_can_reuse() {
        for entry in MODEL_CATALOG {
            let label = llamacpp_source_label(entry);
            if let Some(gguf) = entry.ollama_gguf_names.first() {
                assert!(label.contains(gguf), "{label}");
            }
            if let Some(bare) = entry.ollama_name {
                if !entry.ollama_gguf_names.is_empty() {
                    assert!(
                        !label.contains(&format!("Ollama {bare} ")),
                        "{label} names the manifest-list tag"
                    );
                }
            }
            assert!(label.starts_with("Hugging Face"), "{label}");
        }
    }
}

#[cfg(test)]
mod ollama_model_choice_tests {
    use super::{
        ollama_recommendation_note, short_model_list, suggested_models, OLLAMA_CLOUD_MODEL_HINT,
    };
    use biorouter::providers::ollama::{OLLAMA_DEFAULT_MODEL, OLLAMA_RECOMMENDED_MODELS};

    fn tags(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    /// Ollama's list is whatever was pulled, sorted by name, and the dialog
    /// pre-selects its first entry. Here that was `deepseek-r1:8b`, a model that
    /// never calls tools (2026-10 protocol survey); the supported pair must come
    /// first, default first, quantization variants included.
    #[test]
    fn the_supported_ollama_models_come_first_and_the_default_is_preselected() {
        let pulled = tags(&[
            "deepseek-r1:8b",
            "gemma4:31b",
            "llama3.1:8b",
            "qwen3.8:27b",
            "qwen3.8:27b-q4_K_M",
            "qwen3.8:latest",
        ]);
        let suggested = suggested_models("ollama", &pulled, &[]);
        let names: Vec<&str> = suggested.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(names, ["qwen3.8:27b", "qwen3.8:27b-q4_K_M", "gemma4:31b"]);
        assert!(suggested
            .iter()
            .all(|(_, hint)| hint.starts_with("Recommended")));

        let items = short_model_list("ollama", &pulled, &suggested);
        assert_eq!(items[0].0, OLLAMA_DEFAULT_MODEL);
        assert_eq!(
            items.len(),
            pulled.len(),
            "every pulled model is still offered"
        );
        let rest: Vec<&str> = items[3..].iter().map(|(m, _, _)| m.as_str()).collect();
        assert_eq!(rest, ["deepseek-r1:8b", "llama3.1:8b", "qwen3.8:latest"]);
    }

    #[test]
    fn other_providers_keep_their_order() {
        let listed = tags(&["b-model", "a-model"]);
        let suggested = suggested_models("openai", &listed, &tags(&["a-model"]));
        assert_eq!(
            suggested,
            [("a-model".to_string(), "Recommended".to_string())]
        );
        let items = short_model_list("openai", &listed, &suggested);
        let names: Vec<&str> = items.iter().map(|(m, _, _)| m.as_str()).collect();
        assert_eq!(names, ["b-model", "a-model"]);
    }

    #[test]
    fn a_machine_without_either_supported_model_is_told_which_to_pull() {
        let note = ollama_recommendation_note(&tags(&["llama3.1:8b"])).unwrap();
        for recommended in OLLAMA_RECOMMENDED_MODELS {
            assert!(note.contains(recommended.tag), "{note}");
        }
        assert!(
            note.contains(&format!("ollama pull {OLLAMA_DEFAULT_MODEL}")),
            "{note}"
        );
        assert!(ollama_recommendation_note(&tags(&["gemma4:31b"])).is_none());
        assert!(ollama_recommendation_note(&tags(&["qwen3.8:27b-q4_K_M"])).is_none());
    }

    /// `gemma4:31b-cloud` starts with the supported tag plus `-`, and Ollama
    /// sends it to ollama.com. It was labelled "Recommended: Gemma 4 31B",
    /// pre-selected, and the pull advice was suppressed, so Enter saved a model
    /// that sends every prompt off the machine. It is no variant, the advice
    /// stands, and it sorts after every local model with a hint saying where it
    /// runs.
    #[test]
    fn a_cloud_tag_is_never_the_recommended_local_model() {
        let pulled = tags(&["gemma4:31b-cloud", "llama3.1:8b"]);
        assert!(suggested_models("ollama", &pulled, &[]).is_empty());
        assert!(ollama_recommendation_note(&pulled).is_some());

        let items = short_model_list("ollama", &pulled, &[]);
        let names: Vec<&str> = items.iter().map(|(m, _, _)| m.as_str()).collect();
        assert_eq!(names, ["llama3.1:8b", "gemma4:31b-cloud"]);
        assert!(items[0].2.is_empty());
        assert_eq!(items[1].2, OLLAMA_CLOUD_MODEL_HINT);

        // Next to the supported model, the cloud tag still is not one.
        let pulled = tags(&["gemma4:31b", "gemma4:31b-cloud", "qwen3.8:27b-CLOUD"]);
        let suggested = suggested_models("ollama", &pulled, &[]);
        let names: Vec<&str> = suggested.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(names, ["gemma4:31b"]);
        let items = short_model_list("ollama", &pulled, &suggested);
        assert_eq!(items[0].0, "gemma4:31b");
        assert!(items[1..]
            .iter()
            .all(|(_, _, h)| h == OLLAMA_CLOUD_MODEL_HINT));
    }
}

#[cfg(test)]
mod unlisted_provider_model_tests {
    use super::{
        chat_providers, known_model_items, long_model_list, preselected_model,
        short_model_selection, suggested_models, with_model_in_use, UNLISTED_MODEL_KEY,
    };
    use biorouter::providers::base::ProviderMetadata;
    use biorouter::providers::providers;

    /// The registry's own metadata, the source `biorouter models list` prints.
    async fn meta(name: &str) -> ProviderMetadata {
        providers()
            .await
            .into_iter()
            .find(|(m, _)| m.name == name)
            .map(|(m, _)| m)
            .unwrap_or_else(|| panic!("no registry entry for `{name}`"))
    }

    fn values(items: &[(String, String, String)]) -> Vec<&str> {
        items.iter().map(|(value, _, _)| value.as_str()).collect()
    }

    fn codex_account_models() -> Vec<String> {
        std::iter::once("account-default".to_string())
            .chain((1..12).map(|index| format!("account-model-{index}")))
            .collect()
    }

    #[test]
    fn copilot_cli_account_default_stays_first_above_ten_models() {
        let models = codex_account_models();
        for known in [
            vec!["account-model-2".to_string(), "account-model-1".to_string()],
            Vec::new(),
        ] {
            let suggested = suggested_models("github_copilot", &models, &known);
            assert_eq!(
                suggested[0],
                ("account-default".to_string(), "Recommended".to_string())
            );
            let (items, initial) = long_model_list("github_copilot", &models, &known, None);
            assert_eq!(items[0].0, "account-default");
            assert_eq!(items[1].0, "account-model-1");
            assert_eq!(items.last().unwrap().0, "search_all");
            assert_eq!(initial, None);
        }
    }

    #[test]
    fn copilot_cli_long_account_list_preserves_saved_bindings() {
        let models = codex_account_models();
        let known = vec!["account-model-2".to_string()];
        for saved in ["account-model-11", "claude-opus-4.8", "gpt-5.6-terra"] {
            let (items, initial) = long_model_list("github_copilot", &models, &known, Some(saved));
            assert_eq!(items[0].0, "account-default");
            assert_eq!(initial.as_deref(), Some(saved));
            assert_eq!(items.iter().filter(|(id, _, _)| id == saved).count(), 1);
        }
    }

    #[test]
    fn copilot_cli_short_account_list_keeps_order_and_saved_choice() {
        let models = vec!["claude-sonnet-5.5".to_string(), "gpt-5.3-codex".to_string()];
        let suggested = suggested_models("github_copilot", &models, &["gpt-5.3-codex".to_string()]);
        let (items, initial) = short_model_selection(
            "github_copilot",
            &models,
            &suggested,
            Some("claude-opus-4.8"),
        );
        assert_eq!(
            values(&items),
            ["claude-sonnet-5.5", "claude-opus-4.8", "gpt-5.3-codex"]
        );
        assert_eq!(initial.as_deref(), Some("claude-opus-4.8"));
    }

    #[test]
    fn copilot_cli_empty_account_list_keeps_no_named_fallback_and_saved_explicit_choice() {
        let known = vec!["gpt-5.3-codex".to_string(), "gpt-5.6-terra".to_string()];
        let suggested = suggested_models("github_copilot", &[], &known);
        assert!(suggested.is_empty());
        let (items, initial) = short_model_selection("github_copilot", &[], &suggested, None);
        assert!(
            items.is_empty(),
            "successful empty must not expand capability metadata"
        );
        assert_eq!(initial, None);
        let (items, initial) =
            short_model_selection("github_copilot", &[], &suggested, Some("claude-opus-4.8"));
        assert_eq!(values(&items), ["claude-opus-4.8"]);
        assert_eq!(items[0].2, "In use");
        assert_eq!(initial.as_deref(), Some("claude-opus-4.8"));
    }

    #[test]
    fn codex_live_catalogue_default_stays_first_when_more_than_ten_models_are_available() {
        let models = codex_account_models();
        for known in [
            vec!["account-model-2".to_string(), "account-model-1".to_string()],
            Vec::new(),
        ] {
            let (items, initial) = long_model_list("codex", &models, &known, None);
            assert_eq!(items[0].0, "account-default");
            assert_eq!(items[0].2, "Recommended");
            assert_eq!(items[1].0, "account-model-1");
            assert_eq!(initial, None);
            assert!(values(&items).contains(&"search_all"));
        }
    }

    #[test]
    fn codex_live_catalogue_preserves_a_saved_model_beyond_the_recommended_rows() {
        let models = codex_account_models();
        let known = vec!["account-model-2".to_string()];
        for saved in ["account-model-11", "saved-older-model"] {
            let (items, initial) = long_model_list("codex", &models, &known, Some(saved));
            assert_eq!(items[0].0, "account-default");
            assert!(values(&items).contains(&saved));
            assert_eq!(initial.as_deref(), Some(saved));
            assert_eq!(
                items.iter().filter(|(model, _, _)| model == saved).count(),
                1
            );
        }
    }

    #[test]
    fn codex_live_catalogue_keeps_other_providers_existing_search_and_selection() {
        let models = codex_account_models();
        let known = vec!["account-model-2".to_string()];
        let (items, initial) = long_model_list("example", &models, &known, Some("account-model-2"));
        assert_eq!(items[0].0, "search_all");
        assert_eq!(items[1].0, "account-model-2");
        assert_eq!(initial.as_deref(), Some("account-model-2"));
    }

    #[test]
    fn codex_short_live_catalogue_retains_a_saved_model_missing_from_the_account_list() {
        let models = vec!["account-default".to_string(), "account-model-1".to_string()];
        let suggested = suggested_models("codex", &models, &[]);
        let (items, initial) =
            short_model_selection("codex", &models, &suggested, Some("saved-older-model"));
        assert_eq!(items[0].0, "account-default");
        assert_eq!(items[1].0, "saved-older-model");
        assert_eq!(initial.as_deref(), Some("saved-older-model"));
        assert_eq!(
            items
                .iter()
                .filter(|(model, _, _)| model == "saved-older-model")
                .count(),
            1
        );
    }

    #[test]
    fn codex_short_live_catalogue_keeps_live_order_and_existing_saved_choices() {
        let models = vec!["account-default".to_string(), "account-model-1".to_string()];
        let suggested = suggested_models("codex", &models, &[]);
        for (current, expected) in [
            (None, None),
            (Some("account-model-1"), Some("account-model-1")),
        ] {
            let (items, initial) = short_model_selection("codex", &models, &suggested, current);
            assert_eq!(values(&items), ["account-default", "account-model-1"]);
            assert_eq!(initial.as_deref(), expected);
        }
    }

    /// Phase C, T1 F3: Versa API Bedrock has no model listing, so `configure`
    /// asked for a typed name with Opus 5.5 filled in and never showed Sonnet
    /// 5.5 or Haiku 5.5, which `biorouter models list versa_bedrock` names.
    #[tokio::test]
    async fn configure_offers_versa_bedrocks_sonnet_and_haiku_5_5_with_opus_5_5_first() {
        let meta = meta("versa_bedrock").await;
        let items = known_model_items(&meta).expect("Versa API Bedrock declares its models");
        let offered = values(&items);

        assert_eq!(offered[0], "us.anthropic.claude-opus-5-5");
        assert_eq!(offered[0], meta.default_model);
        assert_eq!(items[0].2, "Recommended");
        for model in [
            "us.anthropic.claude-sonnet-5-5",
            "us.anthropic.claude-haiku-5-5",
        ] {
            assert!(
                offered.contains(&model),
                "{model} is not offered: {offered:?}"
            );
        }
        // Every model `models list` prints, once each, then the typed-name row.
        for known in &meta.known_models {
            assert_eq!(
                offered.iter().filter(|v| **v == known.name).count(),
                1,
                "{} in {offered:?}",
                known.name
            );
        }
        assert_eq!(items.len(), meta.known_models.len() + 1);
        assert_eq!(offered.last(), Some(&UNLISTED_MODEL_KEY));
        assert_eq!(items.last().unwrap().1, "Enter a model not listed...");
        assert!(items[1..].iter().all(|(_, _, hint)| hint.is_empty()));
    }

    /// The other Versa gateway answers the same way: its deployments, its
    /// default first.
    #[tokio::test]
    async fn configure_offers_versa_azures_deployments_with_its_default_first() {
        let meta = meta("versa_azure").await;
        let items = known_model_items(&meta).expect("Versa API Azure declares its models");
        let offered = values(&items);
        assert_eq!(offered[0], meta.default_model);
        assert_eq!(items[0].2, "Recommended");
        let declared: Vec<&str> = meta.known_models.iter().map(|m| m.name.as_str()).collect();
        let mut listed: Vec<&str> = offered[..offered.len() - 1].to_vec();
        listed.sort_unstable();
        let mut expected = declared.clone();
        expected.sort_unstable();
        assert_eq!(listed, expected);
        assert_eq!(offered.last(), Some(&UNLISTED_MODEL_KEY));
    }

    /// Every chat provider: a declared list is offered with the default first
    /// and marked, and a provider that declares none (Ollama) keeps the typed
    /// name.
    #[tokio::test]
    async fn every_provider_that_declares_models_offers_them_default_first() {
        for (meta, _) in chat_providers().await {
            match known_model_items(&meta) {
                None => assert!(meta.known_models.is_empty(), "{}", meta.name),
                Some(items) => {
                    assert!(!meta.known_models.is_empty(), "{}", meta.name);
                    assert_eq!(items[0].0, meta.default_model, "{}", meta.name);
                    assert_eq!(items[0].2, "Recommended", "{}", meta.name);
                    assert_eq!(
                        items.iter().filter(|(_, _, h)| !h.is_empty()).count(),
                        1,
                        "{}: only the default is marked",
                        meta.name
                    );
                    assert_eq!(items.last().unwrap().0, UNLISTED_MODEL_KEY, "{}", meta.name);
                }
            }
        }
        assert!(known_model_items(&meta("ollama").await).is_none());
    }

    /// The list opens on the model in use when it is one of the listed models.
    #[tokio::test]
    async fn the_list_opens_on_the_model_in_use_when_it_is_listed() {
        let items = known_model_items(&meta("versa_bedrock").await).unwrap();
        assert_eq!(
            preselected_model(&items, Some("us.anthropic.claude-haiku-5-5")).as_deref(),
            Some("us.anthropic.claude-haiku-5-5")
        );
        assert_eq!(
            preselected_model(&items, Some("gpt-5.6-sol-2026-07-09")),
            None
        );
        assert_eq!(preselected_model(&items, Some(UNLISTED_MODEL_KEY)), None);
        assert_eq!(preselected_model(&items, None), None);
    }

    /// A model in use that the list does not name (a deployment typed before)
    /// gets a row of its own after the default, and the list opens on it; one
    /// the list names is not repeated.
    #[tokio::test]
    async fn a_model_in_use_the_list_does_not_name_is_offered_after_the_default() {
        let items = known_model_items(&meta("versa_azure").await).unwrap();
        let default = items[0].0.clone();

        let with_custom = with_model_in_use(items.clone(), Some(" my-lab-deployment "));
        assert_eq!(with_custom.len(), items.len() + 1);
        assert_eq!(with_custom[0].0, default, "the default stays first");
        assert_eq!(
            with_custom[1],
            (
                "my-lab-deployment".to_string(),
                "my-lab-deployment".to_string(),
                "In use".to_string()
            )
        );
        assert_eq!(
            preselected_model(&with_custom, Some("my-lab-deployment")).as_deref(),
            Some("my-lab-deployment")
        );

        let listed = items[1].0.clone();
        assert_eq!(with_model_in_use(items.clone(), Some(&listed)), items);
        assert_eq!(with_model_in_use(items.clone(), None), items);
        assert_eq!(with_model_in_use(items.clone(), Some("  ")), items);
        assert_eq!(
            with_model_in_use(items.clone(), Some(UNLISTED_MODEL_KEY)),
            items
        );
    }

    /// Both dialogs reach the list when the provider has no listing. Read at
    /// the source, because the dialog is a terminal conversation.
    #[test]
    fn both_model_prompts_offer_the_declared_models() {
        let source = include_str!("configure.rs");
        for dialog in [
            "pub async fn configure_provider_dialog()",
            "pub async fn configure_default_worker_dialog()",
        ] {
            let (_, body) = source.split_once(dialog).expect("the dialog");
            let (body, _) = body.split_once("\n}\n").expect("its end");
            assert!(
                body.contains("prompt_for_unlisted_provider_model("),
                "{dialog} asks for a typed model without offering the declared ones"
            );
            assert!(
                body.contains("select_model_from_list(&models, provider_meta, current"),
                "{dialog}'s listed models do not open on the model in use"
            );
            assert!(
                !body.contains("cliclack::input(\"Enter a model"),
                "{dialog}"
            );
            assert!(
                !body.contains("cliclack::input(\"Enter the model"),
                "{dialog}"
            );
        }
    }
}

/// Issue #526: `biorouter configure --provider <provider> --secret-stdin`, the
/// host-side way to enter a provider's key that never echoes it, never puts it
/// on a command line, and saves it only once the provider accepted it.
#[cfg(test)]
mod issue_526_secret_entry_tests {
    use super::{
        handle_configure_provider_secret, parse_secret_lines, read_secret_input,
        save_checked_secrets, secret_settings_taken, SecretCheckFailure, TypedSetting,
        SECRET_ENTRY_IS_A_PERSONS, SECRET_INPUT_MAX_BYTES,
    };
    use crate::commands::needs_terminal::{NeedsTerminal, AGENT_SHELL_MARKER};
    use biorouter::config::Config;
    use biorouter::providers::base::{ConfigKey, ProviderMetadata};

    const KEY: &str = "ISSUE526_PROBE_API_KEY";
    const ORG: &str = "ISSUE526_PROBE_ORG_KEY";
    const HOST: &str = "ISSUE526_PROBE_HOST";

    fn provider() -> ProviderMetadata {
        let mut metadata = ProviderMetadata::empty();
        metadata.name = "issue526_probe".into();
        metadata.display_name = "Probe".into();
        metadata.config_keys = vec![
            ConfigKey::new(KEY, true, true, None),
            ConfigKey::new(ORG, false, true, None),
            ConfigKey::new(HOST, true, false, Some("https://probe.example.test")),
        ];
        metadata
    }

    fn parse(input: &str) -> anyhow::Result<Vec<TypedSetting>> {
        let metadata = provider();
        let accepted = secret_settings_taken(&metadata).unwrap();
        parse_secret_lines(input.as_bytes(), &metadata, &accepted)
    }

    fn leaked_config(dir: &tempfile::TempDir) -> &'static Config {
        Box::leak(Box::new(
            Config::new_with_file_secrets(
                dir.path().join("config.yaml"),
                dir.path().join("secrets.yaml"),
            )
            .unwrap(),
        ))
    }

    #[test]
    fn issue_526_name_value_lines_give_the_providers_secrets() {
        let typed = parse(&format!(
            "# from a file only this account reads\n\n  {KEY} = sk-one=two  \r\n{ORG}=org-7\n"
        ))
        .unwrap();
        assert_eq!(
            typed,
            vec![
                TypedSetting {
                    name: KEY.into(),
                    secret: true,
                    value: "sk-one=two".into(),
                },
                TypedSetting {
                    name: ORG.into(),
                    secret: true,
                    value: "org-7".into(),
                },
            ]
        );
    }

    /// Only the provider's secrets: a setting that decides where a key is sent
    /// stays with `biorouter configure` (and a served page's proof, SD-18), and
    /// a refusal never repeats the line, which may be a key holding an `=`.
    #[test]
    fn issue_526_a_refused_line_is_named_by_number_and_never_repeated() {
        for (input, expect) in [
            (format!("{HOST}=https://elsewhere.example"), "line 1"),
            ("sk-live-AbC=def".to_string(), "line 1"),
            ("# note\nsk-live-AbCdef".to_string(), "line 2"),
            (format!("{KEY}=   "), "no value"),
            (format!("{KEY}=a\u{7}b"), "control character"),
            (format!("{KEY}=one\n{KEY}=two"), "more than once"),
            ("\n# only a comment\n".to_string(), "gave no key"),
        ] {
            let error = parse(&input).unwrap_err().to_string();
            assert!(error.contains(expect), "{input:?}: {error}");
            assert!(error.contains("nothing was saved"), "{error}");
            for leaked in ["elsewhere", "sk-live", "AbC", "def"] {
                assert!(!error.contains(leaked), "{input:?} leaked into: {error}");
            }
        }
        let error = parse(&format!("{HOST}=x")).unwrap_err().to_string();
        assert!(error.contains(KEY) && error.contains(ORG), "{error}");
        assert!(!error.contains(HOST), "{error}");
    }

    #[test]
    fn issue_526_input_is_bounded_and_must_be_text() {
        let limit = SECRET_INPUT_MAX_BYTES as usize;
        assert_eq!(
            read_secret_input(&vec![b'#'; limit][..]).unwrap().len(),
            limit
        );
        let error = read_secret_input(&vec![b'#'; limit + 1][..])
            .unwrap_err()
            .to_string();
        assert!(error.contains("64 KiB"), "{error}");

        let metadata = provider();
        let accepted = secret_settings_taken(&metadata).unwrap();
        let error = parse_secret_lines(&[0xff, 0xfe, b'\n'], &metadata, &accepted)
            .unwrap_err()
            .to_string();
        assert!(error.contains("UTF-8"), "{error}");
    }

    #[test]
    fn issue_526_a_provider_without_a_key_says_what_to_do_instead() {
        let mut keyless = provider();
        keyless.config_keys = vec![ConfigKey::new(HOST, true, false, None)];
        let error = secret_settings_taken(&keyless).unwrap_err().to_string();
        assert!(error.contains("takes no API key"), "{error}");
        assert!(error.contains("biorouter models set --provider issue526_probe"));

        let mut signed_in = provider();
        signed_in.config_keys = vec![ConfigKey::new_oauth(KEY, true, true, None)];
        let error = secret_settings_taken(&signed_in).unwrap_err().to_string();
        assert!(error.contains("signs in through a browser"), "{error}");
    }

    /// The check sees the typed key in force; a refusal, an unreachable
    /// provider or a timeout writes nothing, and the sentence never carries the
    /// typed key, even where it quotes the provider.
    #[tokio::test]
    async fn issue_526_nothing_is_saved_unless_the_provider_check_passes() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = leaked_config(&dir);
        config.set_secret(KEY, &"working-key").unwrap();
        let metadata = provider();
        let typed = vec![TypedSetting {
            name: KEY.into(),
            secret: true,
            value: "typed-526-key".into(),
        }];

        for failure in [
            SecretCheckFailure::Rejected,
            SecretCheckFailure::Unreachable,
            SecretCheckFailure::TimedOut,
            SecretCheckFailure::Unbuildable("bad url https://x.test/?key=typed-526-key".into()),
        ] {
            let error = save_checked_secrets(config, &metadata, &typed, async {
                let seen: String = Config::global().get_secret(KEY).unwrap();
                assert_eq!(seen, "typed-526-key", "the check runs with the typed key");
                Err(failure.clone())
            })
            .await
            .unwrap_err()
            .to_string();
            assert!(error.contains("nothing was saved"), "{error}");
            assert!(!error.contains("typed-526-key"), "{failure:?}: {error}");
            assert_eq!(
                config.get_secret::<String>(KEY).unwrap(),
                "working-key",
                "{failure:?} replaced the working key"
            );
        }

        let saved = save_checked_secrets(config, &metadata, &typed, async { Ok(()) })
            .await
            .unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].0, KEY);
        assert!(!saved[0].1.contains("typed-526-key"));
        assert_eq!(config.get_secret::<String>(KEY).unwrap(), "typed-526-key");
        assert!(
            config.get_param::<String>(HOST).is_err(),
            "nothing but the typed secret is written"
        );
    }

    /// Accident prevention, as `biorouter crew` does it: a key typed for an
    /// agent would pass through its model, so the command refuses in an
    /// agent's shell before it reads anything.
    #[tokio::test]
    async fn issue_526_an_agent_shell_is_refused_before_anything_is_read() {
        let _env = env_lock::lock_env([(AGENT_SHELL_MARKER, Some("1"))]);
        let error = handle_configure_provider_secret("issue526_probe".into())
            .await
            .unwrap_err();
        let refusal = error
            .downcast_ref::<NeedsTerminal>()
            .expect("the typed refusal main maps to exit 2");
        assert_eq!(refusal.to_string(), SECRET_ENTRY_IS_A_PERSONS);
    }
}
