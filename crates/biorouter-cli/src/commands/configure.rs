use crate::workflows::github_workflow::BIOROUTER_WORKFLOW_GITHUB_REPO_CONFIG_KEY;
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
use biorouter::providers::{create, providers, retry_operation, RetryConfig};
use biorouter::session::SessionType;
use cliclack::spinner;
use console::style;
use serde_json::Value;
use std::collections::HashMap;

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
     variable, e.g. `OPENAI_API_KEY`.";

pub async fn handle_configure() -> anyhow::Result<()> {
    configure(Config::global(), super::needs_terminal::prompt_can_run()).await
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
            let source = e
                .ollama_name
                .map(|name| format!("Ollama {name}"))
                .unwrap_or_else(|| "Hugging Face fallback".to_string());
            format!("{} · {} · {}", e.display_name, e.download_size, source)
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
        "The first run uses Ollama's local model store when available; otherwise a fallback GGUF download can take several minutes.",
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
            "biorouter settings",
            "Set the biorouter mode, Tool Output, Tool Permissions, Experiment, biorouter workflow github repo and more",
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

fn select_model_from_list(
    models: &[String],
    provider_meta: &biorouter::providers::base::ProviderMetadata,
) -> anyhow::Result<String> {
    const MAX_MODELS: usize = 10;
    const UNLISTED_MODEL_KEY: &str = "__unlisted__";

    // Smart model selection:
    // If we have more than MAX_MODELS models, show the recommended models with additional search option.
    // Otherwise, show all models without search.
    if models.len() > MAX_MODELS {
        let recommended_models: Vec<String> = provider_meta
            .known_models
            .iter()
            .map(|m| m.name.clone())
            .filter(|name| models.contains(name))
            .collect();

        if !recommended_models.is_empty() {
            let mut model_items: Vec<(String, String, &str)> = recommended_models
                .iter()
                .map(|m| (m.clone(), m.clone(), "Recommended"))
                .collect();

            model_items.insert(
                0,
                (
                    "search_all".to_string(),
                    "Search all models...".to_string(),
                    "Search complete model list",
                ),
            );

            if provider_meta.allows_unlisted_models {
                model_items.push((
                    UNLISTED_MODEL_KEY.to_string(),
                    "Enter a model not listed...".to_string(),
                    "",
                ));
            }

            let selection = cliclack::select("Select a model:")
                .items(&model_items)
                .interact()?;

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
        let mut model_items: Vec<(String, String, &str)> =
            models.iter().map(|m| (m.clone(), m.clone(), "")).collect();

        if provider_meta.allows_unlisted_models {
            model_items.push((
                UNLISTED_MODEL_KEY.to_string(),
                "Enter a model not listed...".to_string(),
                "",
            ));
        }

        let selection = cliclack::select("Select a model:")
            .items(&model_items)
            .interact()?;

        if selection == UNLISTED_MODEL_KEY {
            prompt_unlisted_model(provider_meta)
        } else {
            Ok(selection)
        }
    }
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

fn try_store_secret(config: &Config, key_name: &str, value: String) -> anyhow::Result<bool> {
    match config.set_secret(key_name, &value) {
        Ok(_) => Ok(true),
        Err(ConfigError::FallbackToFileStorage) => Ok(true),
        Err(e) => {
            cliclack::outro(style(format!(
                "Failed to store {} securely: {}. Please ensure your system's secure storage is accessible. Alternatively you can run with BIOROUTER_DISABLE_KEYRING=true or set the key in your environment variables",
                key_name, e
            )).on_red().white())?;
            Ok(false)
        }
    }
}

#[allow(clippy::too_many_lines)]
pub async fn configure_provider_dialog() -> anyhow::Result<bool> {
    // Get global config instance
    let config = Config::global();

    // Get all available providers and their metadata
    let mut available_providers = providers().await;

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
                if cliclack::confirm("Would you like to save this value to your keyring?")
                    .initial_value(true)
                    .interact()?
                {
                    if key.secret {
                        if !try_store_secret(config, &key.name, env_value)? {
                            return Ok(false);
                        }
                    } else {
                        config.set_param(&key.name, &env_value)?;
                    }
                    let _ = cliclack::log::info(format!("Saved {} to {}", key.name, config.path()));
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

                                if key.secret {
                                    if !try_store_secret(config, &key.name, value)? {
                                        return Ok(false);
                                    }
                                } else {
                                    config.set_param(&key.name, &value)?;
                                }
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

                            if key.secret {
                                if !try_store_secret(config, &key.name, value)? {
                                    return Ok(false);
                                }
                            } else {
                                config.set_param(&key.name, &value)?;
                            }
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
            if key.secret {
                if !try_store_secret(config, &key.name, value)? {
                    return Ok(false);
                }
            } else {
                config.set_param(&key.name, &value)?;
            }
        }
    }

    let spin = spinner();
    spin.start("Attempting to fetch supported models...");
    let models_res = {
        let temp_model_config = ModelConfig::new(&provider_meta.default_model)?;
        let temp_provider = create(provider_name, temp_model_config).await?;
        retry_operation(&RetryConfig::default(), || async {
            temp_provider.fetch_recommended_models().await
        })
        .await
    };
    spin.stop(style("Model fetch complete").green());

    // Select a model: on fetch error show styled error and abort; if Some(models), show list; if None, free-text input
    let model: String = match models_res {
        Err(e) => {
            // Provider hook error
            cliclack::outro(style(e.to_string()).on_red().white())?;
            return Ok(false);
        }
        Ok(Some(models)) => select_model_from_list(&models, provider_meta)?,
        Ok(None) => {
            let default_model =
                std::env::var("BIOROUTER_MODEL").unwrap_or(provider_meta.default_model.clone());
            cliclack::input("Enter a model from that provider:")
                .default_input(&default_model)
                .interact()?
        }
    };

    // Test the configuration
    let spin = spinner();
    spin.start("Checking your configuration...");

    let toolshim_enabled = std::env::var("BIOROUTER_TOOLSHIM")
        .map(|val| val == "1" || val.to_lowercase() == "true")
        .unwrap_or(false);
    let toolshim_model = std::env::var("BIOROUTER_TOOLSHIM_OLLAMA_MODEL").ok();

    match test_provider_configuration(provider_name, &model, toolshim_enabled, toolshim_model).await
    {
        Ok(()) => {
            config.set_biorouter_provider_and_model(provider_name, &model)?;
            print_config_file_saved()?;
            Ok(true)
        }
        Err(e) => {
            spin.stop(style(e.to_string()).red());
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
            "Interactive charts, diagrams, networks, maps and scientific plots, rendered inline.",
        ),
        (
            "computercontroller",
            "Computer Controller",
            "Control desktop apps, scrape web pages, and work with local files.",
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
        .placeholder("npx -y @block/gdrive")
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
            "biorouter mode",
            "Configure biorouter mode",
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
            "lead_worker",
            "Lead/Worker Model",
            "Use a stronger lead model for planning and a worker model for execution",
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
            "biorouter workflow github repo",
            "biorouter will pull workflows from this repo if not found locally.",
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
        "lead_worker" => {
            configure_lead_worker_dialog().await?;
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

/// Configure lead/worker mode: a stronger "lead" model handles the first
/// turns of a session (and failure recovery), then the default (worker)
/// model takes over. Mirrors the GUI's Lead/Worker settings; previously this
/// was only reachable through BIOROUTER_LEAD_* environment variables.
pub async fn configure_lead_worker_dialog() -> anyhow::Result<()> {
    let config = Config::global();

    let current_lead_model: Option<String> = config.get_param("BIOROUTER_LEAD_MODEL").ok();
    let enabled = current_lead_model.is_some();

    if enabled {
        let _ = cliclack::log::info(format!(
            "Lead/worker mode is currently ON (lead model: {})",
            current_lead_model.as_deref().unwrap_or("unknown")
        ));
    } else {
        let _ = cliclack::log::info(
            "Lead/worker mode runs a stronger lead model for the first turns of a chat, then switches to your default (worker) model.",
        );
    }

    let action = cliclack::select("What would you like to do?")
        .item(
            "configure",
            if enabled {
                "Update lead/worker settings"
            } else {
                "Enable lead/worker mode"
            },
            "",
        )
        .item(
            "disable",
            "Disable lead/worker mode",
            if enabled { "" } else { "(already off)" },
        )
        .interact()?;

    if action == "disable" {
        for key in [
            "BIOROUTER_LEAD_MODEL",
            "BIOROUTER_LEAD_PROVIDER",
            "BIOROUTER_LEAD_TURNS",
            "BIOROUTER_LEAD_FAILURE_THRESHOLD",
            "BIOROUTER_LEAD_FALLBACK_TURNS",
        ] {
            let _ = config.delete(key);
        }
        let _ = cliclack::log::success("Lead/worker mode disabled.");
        return Ok(());
    }

    // Pick the lead provider, defaulting to the configured lead provider or
    // the main provider.
    let mut available_providers = providers().await;
    available_providers.sort_by(|a, b| a.0.display_name.cmp(&b.0.display_name));
    let provider_items: Vec<(&String, &str, &str)> = available_providers
        .iter()
        .map(|(p, _)| (&p.name, p.display_name.as_str(), p.description.as_str()))
        .collect();
    let default_provider: String = config
        .get_param("BIOROUTER_LEAD_PROVIDER")
        .ok()
        .or_else(|| config.get_biorouter_provider().ok())
        .unwrap_or_default();
    let provider_name = cliclack::select("Which provider should the lead model use?")
        .initial_value(&default_provider)
        .items(&provider_items)
        .interact()?;
    let (provider_meta, _) = available_providers
        .iter()
        .find(|(p, _)| &p.name == provider_name)
        .expect("Selected provider must exist in metadata");

    // Pick the lead model, fetching the provider's model list when possible.
    let spin = spinner();
    spin.start("Fetching models for the lead provider...");
    let models_res = {
        let temp_model_config = ModelConfig::new(&provider_meta.default_model)?;
        let temp_provider = create(provider_name, temp_model_config).await?;
        retry_operation(&RetryConfig::default(), || async {
            temp_provider.fetch_recommended_models().await
        })
        .await
    };
    spin.stop(style("Model fetch complete").green());
    let model: String = match models_res {
        Ok(Some(models)) if !models.is_empty() => select_model_from_list(&models, provider_meta)?,
        _ => {
            let default_model =
                current_lead_model.unwrap_or_else(|| provider_meta.default_model.clone());
            cliclack::input("Enter the lead model name:")
                .default_input(&default_model)
                .interact()?
        }
    };

    let prompt_turns = |question: &str, key: &str, default: u32| -> anyhow::Result<u32> {
        let current: u32 = config.get_param(key).unwrap_or(default);
        let value: String = cliclack::input(question)
            .default_input(&current.to_string())
            .validate(|input: &String| {
                if input.trim().parse::<u32>().is_ok() {
                    Ok(())
                } else {
                    Err("Please enter a positive whole number")
                }
            })
            .interact()?;
        Ok(value.trim().parse::<u32>().expect("validated above"))
    };

    let lead_turns = prompt_turns(
        "Initial turns handled by the lead model:",
        "BIOROUTER_LEAD_TURNS",
        3,
    )?;
    let failure_threshold = prompt_turns(
        "Consecutive worker failures before falling back to the lead model:",
        "BIOROUTER_LEAD_FAILURE_THRESHOLD",
        2,
    )?;
    let fallback_turns = prompt_turns(
        "Turns the lead model handles during a fallback:",
        "BIOROUTER_LEAD_FALLBACK_TURNS",
        2,
    )?;

    config.set_param("BIOROUTER_LEAD_MODEL", &model)?;
    config.set_param("BIOROUTER_LEAD_PROVIDER", provider_name)?;
    config.set_param("BIOROUTER_LEAD_TURNS", lead_turns)?;
    config.set_param("BIOROUTER_LEAD_FAILURE_THRESHOLD", failure_threshold)?;
    config.set_param("BIOROUTER_LEAD_FALLBACK_TURNS", fallback_turns)?;

    let worker_model: String = config
        .get_biorouter_model()
        .unwrap_or_else(|_| "your default model".to_string());
    let _ = cliclack::log::success(format!(
        "Lead/worker mode enabled. {} ({}) leads, {} handles the rest.",
        model, provider_meta.display_name, worker_model
    ));
    Ok(())
}

pub fn configure_biorouter_mode_dialog() -> anyhow::Result<()> {
    let config = Config::global();

    if std::env::var("BIOROUTER_MODE").is_ok() {
        let _ = cliclack::log::info("Notice: BIOROUTER_MODE environment variable is set and will override the configuration here.");
    }

    let mode = cliclack::select("Which biorouter mode would you like to configure?")
        .item(
            BioRouterMode::Auto,
            "Auto Mode",
            "Full file modification, extension usage, edit, create and delete files freely"
        )
        .item(
            BioRouterMode::Approve,
            "Approve Mode",
            "All tools, extensions and file modifications will require human approval"
        )
        .item(
            BioRouterMode::SmartApprove,
            "Smart Approve Mode",
            "Editing, creating, deleting files and using extensions will require human approval"
        )
        .item(
            BioRouterMode::Chat,
            "Chat Mode",
            "Engage with the selected provider without using tools, extensions, or file modification"
        )
        .interact()?;

    config.set_biorouter_mode(mode)?;
    let msg = match mode {
        BioRouterMode::Auto => "Set to Auto Mode - full file modification enabled",
        BioRouterMode::Approve => {
            "Set to Approve Mode - all tools and modifications require approval"
        }
        BioRouterMode::SmartApprove => "Set to Smart Approve Mode - modifications require approval",
        BioRouterMode::Chat => "Set to Chat Mode - no tools or modifications enabled",
    };
    cliclack::outro(msg)?;
    Ok(())
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

    let current_max_turns: u32 = config.get_param("BIOROUTER_MAX_TURNS").unwrap_or(1000);

    let max_turns_input: String =
        cliclack::input("Set maximum number of agent turns without user input:")
            .placeholder(&current_max_turns.to_string())
            .default_input(&current_max_turns.to_string())
            .validate(|input: &String| match input.parse::<u32>() {
                Ok(value) => {
                    if value < 1 {
                        Err("Value must be at least 1")
                    } else {
                        Ok(())
                    }
                }
                Err(_) => Err("Please enter a valid number"),
            })
            .interact()?;

    let max_turns: u32 = max_turns_input.parse()?;
    config.set_param("BIOROUTER_MAX_TURNS", max_turns)?;

    cliclack::outro(format!(
        "Set maximum turns to {} - biorouter will ask for input after {} consecutive actions",
        max_turns, max_turns
    ))?;

    Ok(())
}

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
