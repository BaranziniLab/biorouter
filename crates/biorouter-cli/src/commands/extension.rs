//! `biorouter extension` subcommands — install a `.brxt` bundle, list installed
//! extensions, and remove one. Mirrors the desktop GUI's `.brxt` flow: extract
//! into `~/.config/biorouter/extensions/<name>/`, build the Python venv with
//! `uv sync`, store any secret env vars in the keyring, and register a stdio
//! extension that launches via `uv run --directory <dir> <entry_point>`.

use std::collections::HashMap;
use std::fs;
use std::io::{IsTerminal, Read};
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use biorouter::agents::extension::ExtensionConfig;
use biorouter::config::extensions::{get_all_extensions, name_to_key, remove_extension};
use biorouter::config::paths::Paths;
use biorouter::extension_install::{
    claim, BrxtEnvVar, ClaimSource, CredentialPolicy, ExtensionInstallTransaction, InstallClaim,
    InstallReport, InstallSource, InstallState,
};
use console::{style, Color};

const ACCENT: Color = Color::Color256(137);

fn extensions_root() -> PathBuf {
    Paths::config_dir().join("extensions")
}

/// Human-readable transport label for an extension config.
fn kind_str(c: &ExtensionConfig) -> &'static str {
    match c {
        ExtensionConfig::Sse { .. } => "sse",
        ExtensionConfig::StreamableHttp { .. } => "http",
        ExtensionConfig::Stdio { .. } => "stdio",
        ExtensionConfig::Builtin { .. } => "builtin",
        ExtensionConfig::Platform { .. } => "platform",
        ExtensionConfig::Frontend { .. } => "frontend",
        ExtensionConfig::InlinePython { .. } => "inline-python",
    }
}

/// Parse repeated `KEY=VALUE` flags into a map, erroring on malformed entries.
fn parse_kv(pairs: &[String]) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for pair in pairs {
        let (k, v) = pair
            .split_once('=')
            .ok_or_else(|| anyhow!("Invalid KEY=VALUE pair: '{}'", pair))?;
        map.insert(k.trim().to_string(), v.to_string());
    }
    Ok(map)
}

// ──────────────────────────────────────────────────────────────────────────────
// install
// ──────────────────────────────────────────────────────────────────────────────

/// Install a `.brxt` bundle.
///
/// ⚠ **This used to print a warning about a missing required value and register
/// the extension anyway** (#117). The result started, failed to authenticate,
/// and reported success — which is fine when a human read the warning scrolling
/// past, and is exactly wrong when an agent ran the command and reported the
/// exit code. There is no "install it broken" path any more:
///
/// * at a terminal, the missing values are asked for, with echo off for
///   anything the manifest declares secret;
/// * unattended, the install stops and says which key names it needs, and the
///   extension is not registered.
///
/// The heavy lifting is [`biorouter::extension_install`], so this and the
/// desktop and an agent-driven install are the same transaction with different
/// front doors.
pub async fn handle_install(
    path: PathBuf,
    env_flags: Vec<String>,
    secret_flags: Vec<String>,
    secret_stdin: bool,
    no_enable: bool,
) -> Result<()> {
    let mut supplied = parse_kv(&env_flags)?;
    let from_flags = parse_kv(&secret_flags)?;
    if !from_flags.is_empty() {
        // ⚠ Kept working, never recommended. A value in `--secret` is in the
        // shell history and in `ps` output for the life of the process, and the
        // caller has already typed it by the time we get here — refusing would
        // only cost them the install without un-exposing anything. The warning
        // names the two paths that do not have the problem.
        println!(
            "  {} {} is visible in your shell history and to `ps`. \
             Run without it to be prompted with echo off, or pipe `KEY=VALUE` lines \
             into `--secret-stdin`.",
            style("⚠").yellow(),
            style("--secret").yellow(),
        );
    }
    supplied.extend(from_flags);
    supplied.extend(read_secret_stdin(secret_stdin)?);

    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    let policy = if interactive {
        CredentialPolicy::Prompt(Box::new(prompt_for_values))
    } else {
        CredentialPolicy::Refuse
    };

    let spinner = cliclack::spinner();
    spinner.start("installing…");
    let report = ExtensionInstallTransaction::new(InstallSource::LocalFile { path })
        .with_values(supplied)
        .enabled(!no_enable)
        .run(policy, None)
        .await;
    spinner.stop("");

    report_install(&report, interactive)
}

/// Read `KEY=VALUE` lines from stdin when `--secret-stdin` was passed.
///
/// The unattended answer to "how do I configure a secret without putting it in
/// `ps`". Reading the whole of stdin is why it is opt-in: an interactive run
/// would block on it forever.
fn read_secret_stdin(enabled: bool) -> Result<HashMap<String, String>> {
    if !enabled {
        return Ok(HashMap::new());
    }
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("reading KEY=VALUE lines from stdin")?;
    let pairs: Vec<String> = buf
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();
    parse_kv(&pairs)
}

/// Ask at the terminal, with echo off for anything declared secret.
fn prompt_for_values(vars: &[BrxtEnvVar]) -> Result<HashMap<String, String>> {
    let mut values = HashMap::new();
    for var in vars {
        let label = if var.required {
            format!("{} (required)", var.key)
        } else {
            format!("{} (optional — Enter to skip)", var.key)
        };
        let help = if var.description.is_empty() {
            String::new()
        } else {
            format!("\n  {}", style(&var.description).dim())
        };
        print!("{help}");

        let entered: String = if var.secret {
            // Echo off. The one property that makes a terminal safe for this:
            // nothing is written to the screen, so nothing is in the scrollback,
            // and nothing reaches the shell's history file.
            // `required` is enforced below rather than by the widget: an
            // OPTIONAL secret must be skippable with Enter, and the transaction
            // re-checks every required key before it registers anything, so a
            // widget-level rule here would only be a second, divergent copy.
            cliclack::password(label).mask('•').interact()?
        } else {
            cliclack::input(label)
                .default_input(var.default.as_deref().unwrap_or(""))
                .required(false)
                .interact()?
        };
        if !entered.trim().is_empty() {
            values.insert(var.key.clone(), entered);
        }
    }
    Ok(values)
}

/// Say what happened. `needs_credentials` is a first-class result with a
/// non-zero exit, so a script — or an agent — cannot read it as success.
fn report_install(report: &InstallReport, interactive: bool) -> Result<()> {
    match &report.state {
        InstallState::Attached | InstallState::Installed => {
            let state = if report.enabled {
                "installed and enabled"
            } else {
                "installed"
            };
            println!(
                "  {} {} {}",
                style("✓").green(),
                style(report.display_name.as_deref().unwrap_or("extension"))
                    .fg(ACCENT)
                    .bold(),
                style(state).dim()
            );
            if !report.configured_keys.is_empty() {
                // NAMES only — this line is read by whoever ran the command,
                // which in an agent-driven install is a model.
                println!(
                    "  {} configured {}",
                    style("·").dim(),
                    style(report.configured_keys.join(", ")).dim()
                );
            }
            Ok(())
        }
        InstallState::NeedsCredentials { keys } => {
            let name = report.display_name.as_deref().unwrap_or("This extension");
            bail!(
                "{name} needs {} before it can run, and this run has no terminal to ask on.\n  \
                 Missing: {}\n  \
                 Configure them by re-running at a terminal, or pipe `KEY=VALUE` lines in:\n    \
                 printf '%s\\n' 'KEY=…' | biorouter extension install <bundle> --secret-stdin\n  \
                 Do not pass a credential as a command-line argument: it is visible to `ps` \
                 and lands in your shell history.\n  \
                 The extension was NOT registered — an extension that cannot authenticate is \
                 worse than one that is missing.",
                if keys.len() == 1 { "a value" } else { "values" },
                keys.join(", "),
            )
        }
        InstallState::Cancelled => {
            if interactive {
                println!(
                    "  {} install cancelled; nothing was registered",
                    style("·").dim()
                );
                Ok(())
            } else {
                bail!("The install was cancelled; nothing was registered.")
            }
        }
        InstallState::Failed { reason } => bail!("{reason}"),
        // Reachable only if a caller polls a report mid-run, which this
        // command does not do.
        other => bail!("Install ended in an unexpected state: {other:?}"),
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// configure
// ──────────────────────────────────────────────────────────────────────────────

/// Re-enter an installed extension's credentials without reinstalling it.
///
/// The counterpart to the desktop's Configure action (#117): the same manifest,
/// the same echo-off prompt, the same split of credential-to-keyring versus
/// setting-to-config. Existing values are reported as **configured** and never
/// read back — the prompt offers to replace them, and skipping keeps whatever is
/// already stored.
///
/// ⚠ **There need not be a config entry.** An install that stopped at the
/// credential step never registered one — that is the whole point of the stop —
/// so the `No extension named …` refusal below used to fire on exactly the case
/// the extension-manager tool tells the user to run this command for. A parked
/// [`claim`] is the second place to look, and finishing one means resuming the
/// transaction rather than editing an entry that does not exist.
pub async fn handle_configure(name: String) -> Result<()> {
    let key = name_to_key(&name);
    let entry = get_all_extensions()
        .into_iter()
        .find(|e| name_to_key(&e.config.name()) == key);
    let Some(entry) = entry else {
        let claim = claim::read_claims()
            .into_iter()
            .find(|c| name_to_key(&c.extension_name) == key)
            .ok_or_else(|| anyhow!("No extension named '{name}' is configured"))?;
        return finish_parked_install(&name, claim).await;
    };

    let install_dir = brxt_install_dir(&entry.config)
        .ok_or_else(|| anyhow!("'{name}' was not installed from a .brxt bundle"))?;
    let vars = declared_vars(&install_dir)?;
    if vars.is_empty() {
        println!(
            "  {} {} declares no configurable values",
            style("·").dim(),
            style(&name).bold()
        );
        return Ok(());
    }

    if !std::io::stdin().is_terminal() {
        bail!(
            "`biorouter extension configure` needs a terminal so it can read values with echo off.\n  \
             {} declares: {}",
            name,
            vars.iter()
                .map(|v| v.key.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    // Say which are already set — by NAME. Reading a stored value back to
    // pre-fill a field would put it on screen, which is the one thing the
    // echo-off prompt exists to prevent.
    let configured: Vec<&str> = vars
        .iter()
        .filter(|v| biorouter::extension_install::brxt::secret_already_stored(&v.key))
        .map(|v| v.key.as_str())
        .collect();
    if !configured.is_empty() {
        println!(
            "  {} already configured: {} {}",
            style("·").dim(),
            style(configured.join(", ")).dim(),
            style("(Enter to keep)").dim()
        );
    }

    let values = prompt_for_values(&vars)?;
    if values.is_empty() {
        println!("  {} nothing changed", style("·").dim());
        return Ok(());
    }

    let written = store_configured_values(&entry, &vars, values)?;
    println!(
        "  {} {} {}",
        style("✓").green(),
        style(&name).fg(ACCENT).bold(),
        style(if written.is_empty() {
            "settings updated".to_string()
        } else {
            format!("configured {}", written.join(", "))
        })
        .dim()
    );
    Ok(())
}

/// Finish an install that stopped at the credential step.
///
/// The tree and its built `.venv` are still on disk — that is what the claim
/// points at — so the manifest can be read from there and the values asked for
/// here. What this must NOT do is write them with [`store_configured_values`]:
/// that folds the result into an `ExtensionEntry`, and a parked install has
/// none. The values are handed to a fresh transaction carrying the parked
/// install's own id, which registers the extension for the first time.
async fn finish_parked_install(name: &str, claim: InstallClaim) -> Result<()> {
    let vars = declared_vars(&claim.install_dir)?;
    if vars.is_empty() {
        bail!(
            "'{name}' has an unfinished install at {} but declares no configurable values. \
             Re-run `biorouter extension install` on the bundle.",
            claim.install_dir.display()
        );
    }
    if !std::io::stdin().is_terminal() {
        bail!(
            "`biorouter extension configure` needs a terminal so it can read values with echo off.\n  \
             {} has an unfinished install waiting on: {}",
            name,
            claim.pending_keys.join(", ")
        );
    }

    println!(
        "  {} finishing an install of {} that stopped waiting for {}",
        style("·").dim(),
        style(name).bold(),
        style(claim.pending_keys.join(", ")).dim(),
    );
    let values = prompt_for_values(&vars)?;
    let source = resume_source(name, claim.source)?;
    let report = ExtensionInstallTransaction::new(source)
        .with_install_id(claim.install_id)
        .with_values(values)
        .run(CredentialPolicy::Prompt(Box::new(prompt_for_values)), None)
        .await;
    report_install(&report, true)
}

/// The source a resumed install re-reads its bundle from.
///
/// A marketplace claim carries the URL, which is re-fetchable. A local `.brxt`
/// that has since been moved or deleted is not, and saying *which* file is
/// missing is the difference between an actionable message and "install
/// failed".
fn resume_source(name: &str, source: ClaimSource) -> Result<InstallSource> {
    match source {
        ClaimSource::LocalFile { path } => {
            if !path.exists() {
                bail!(
                    "'{name}' has an unfinished install, but the bundle it came from is gone: {}\n  \
                     Re-run `biorouter extension install <bundle>` with the .brxt file.",
                    path.display()
                );
            }
            Ok(InstallSource::LocalFile { path })
        }
        ClaimSource::Marketplace { registry_id, url } => {
            Ok(InstallSource::Marketplace { registry_id, url })
        }
    }
}

/// Where a `.brxt` extension was unpacked, read off the `--directory` argument
/// its launch command carries.
fn brxt_install_dir(config: &ExtensionConfig) -> Option<PathBuf> {
    let ExtensionConfig::Stdio { args, .. } = config else {
        return None;
    };
    args.iter()
        .position(|a| a == "--directory")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
}

/// The `env_vars` an installed bundle's manifest declares.
fn declared_vars(install_dir: &std::path::Path) -> Result<Vec<BrxtEnvVar>> {
    let manifest_path = install_dir.join("manifest.json");
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?,
    )?;
    Ok(
        serde_json::from_value(manifest.get("env_vars").cloned().unwrap_or_default())
            .unwrap_or_default(),
    )
}

/// Write the entered values and fold the result back into the config entry:
/// credential NAMES into `env_keys`, ordinary settings into `envs`.
///
/// Returns the credential names written. A key promoted to the credential store
/// is removed from `envs`, so a value that used to sit in plaintext does not
/// stay there beside its own replacement.
fn store_configured_values(
    entry: &biorouter::config::extensions::ExtensionEntry,
    vars: &[BrxtEnvVar],
    values: HashMap<String, String>,
) -> Result<Vec<String>> {
    let config = biorouter::config::Config::global();
    let mut written: Vec<String> = Vec::new();
    let mut settings: HashMap<String, String> = HashMap::new();
    for (k, v) in values {
        if vars.iter().any(|var| var.key == k && var.secret) {
            config
                .set_secret(&k, &v)
                .map_err(|e| anyhow!("Failed to store '{k}': {e}"))?;
            written.push(k);
        } else {
            settings.insert(k, v);
        }
    }

    let ExtensionConfig::Stdio {
        name: cfg_name,
        description,
        cmd,
        args,
        envs,
        mut env_keys,
        timeout,
        bundled,
        available_tools,
    } = entry.config.clone()
    else {
        bail!("not a .brxt extension");
    };
    let mut env_map = envs.get_env();
    env_map.extend(settings);
    for k in &written {
        if !env_keys.contains(k) {
            env_keys.push(k.clone());
        }
        env_map.remove(k);
    }
    env_keys.sort();
    env_keys.dedup();

    biorouter::config::extensions::set_extension(biorouter::config::extensions::ExtensionEntry {
        enabled: entry.enabled,
        config: ExtensionConfig::Stdio {
            name: cfg_name,
            description,
            cmd,
            args,
            envs: biorouter::agents::extension::Envs::new(env_map),
            env_keys,
            timeout,
            bundled,
            available_tools,
        },
    });
    written.sort();
    Ok(written)
}

// ──────────────────────────────────────────────────────────────────────────────
// list
// ──────────────────────────────────────────────────────────────────────────────

pub async fn handle_list(format: &str) -> Result<()> {
    let entries = get_all_extensions();

    if format == "json" {
        let items: Vec<_> = entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "name": e.config.name(),
                    "enabled": e.enabled,
                    "type": kind_str(&e.config),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    println!("  {} {}", style("▌").fg(ACCENT), style("Extensions").bold());
    if entries.is_empty() {
        println!("    {}", style("none configured").dim());
        return Ok(());
    }
    let width = entries
        .iter()
        .map(|e| e.config.name().len())
        .max()
        .unwrap_or(0);
    for entry in &entries {
        let dot = if entry.enabled {
            style("●").green().to_string()
        } else {
            style("○").dim().to_string()
        };
        println!(
            "    {} {:<width$}  {}",
            dot,
            style(entry.config.name()).bold(),
            style(kind_str(&entry.config)).dim(),
            width = width
        );
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// remove
// ──────────────────────────────────────────────────────────────────────────────

pub async fn handle_remove(name: String, purge: bool) -> Result<()> {
    let key = name_to_key(&name);
    let existed = get_all_extensions()
        .iter()
        .any(|e| name_to_key(&e.config.name()) == key);
    if !existed {
        bail!(
            "No extension named '{}'. Run `biorouter extension list`.",
            name
        );
    }
    remove_extension(&key);
    println!(
        "  {} removed extension {}",
        style("✓").green(),
        style(&name).bold()
    );

    if purge {
        let dir = extensions_root().join(&name);
        if dir.starts_with(extensions_root()) && dir.exists() {
            fs::remove_dir_all(&dir).ok();
            println!(
                "  {} purged {}",
                style("·").dim(),
                style(dir.display()).dim()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(state: InstallState) -> InstallReport {
        InstallReport {
            install_id: "i-1".to_string(),
            state,
            extension_name: Some("spokeagent".to_string()),
            display_name: Some("SPOKE Agent".to_string()),
            configured_keys: Vec::new(),
            skills: Vec::new(),
            enabled: false,
            operator_pinned_off: false,
        }
    }

    /// ⚠ **The behaviour this replaced was: warn, then register anyway.** An
    /// agent running the command read the exit code, reported success, and left
    /// the user with an extension that starts and cannot authenticate. A
    /// non-zero exit is the whole fix.
    #[test]
    fn a_missing_credential_fails_the_command_and_says_nothing_was_registered() {
        let err = report_install(
            &report(InstallState::NeedsCredentials {
                keys: vec!["SPOKEAGENT_PASSCODE".to_string()],
            }),
            false,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("SPOKEAGENT_PASSCODE"), "{err}");
        assert!(err.contains("NOT registered"), "{err}");
    }

    /// The refusal is read by whoever ran the command — in an agent-driven
    /// install, a model. It must not hand them the one call shape that puts the
    /// credential in `ps` and the shell history.
    #[test]
    fn the_refusal_never_recommends_a_credential_on_the_command_line() {
        let err = report_install(
            &report(InstallState::NeedsCredentials {
                keys: vec!["SPOKEAGENT_PASSCODE".to_string()],
            }),
            false,
        )
        .unwrap_err()
        .to_string();

        assert!(
            !err.contains("--secret SPOKEAGENT_PASSCODE")
                && !err.contains("--secret KEY=VALUE")
                && !err.contains("--env SPOKEAGENT_PASSCODE"),
            "the refusal recommended a command-line credential: {err}"
        );
        assert!(err.contains("--secret-stdin"), "{err}");
        assert!(err.contains("visible to `ps`"), "{err}");
    }

    #[test]
    fn a_successful_install_is_not_an_error() {
        let mut ok = report(InstallState::Attached);
        ok.enabled = true;
        assert!(report_install(&ok, true).is_ok());
    }

    /// Reading stdin is opt-in because an interactive run would block on it
    /// forever.
    #[test]
    fn stdin_is_not_read_unless_asked_for() {
        assert!(read_secret_stdin(false).unwrap().is_empty());
    }

    /// ⚠ **The recovery route the extension-manager tool advertises.** An
    /// install that stops at the credential step never registers a config
    /// entry, so `configure` used to bail with "No extension named …" on
    /// exactly the case it is told to users for. The fallback must find the
    /// parked claim and resolve its tree.
    ///
    /// Asserted through the no-terminal refusal: a test process has no tty, so
    /// reaching the "needs a terminal" message — naming the pending key read
    /// out of the claim's own tree — proves the install dir was resolved.
    #[tokio::test]
    async fn configure_falls_back_to_a_parked_claim() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let _guard = crate::test_sandbox::relocate_path_root(tmp.path());
        // Prove the fixture is pointed at the temp tree before anything is
        // written or deleted: the real config root holds live extensions.
        for dir in [claim::claims_dir(), extensions_root()] {
            assert!(
                dir.starts_with(tmp.path()),
                "the fixture resolved to {} — refusing to write there",
                dir.display()
            );
        }

        // A tree with a manifest, and a claim pointing at it. No config entry —
        // that is the state a parked install leaves behind.
        let name = "parkedfixture";
        let tree = extensions_root().join(name);
        fs::create_dir_all(&tree).unwrap();
        fs::write(
            tree.join("manifest.json"),
            serde_json::json!({
                "name": name,
                "env_vars": [{
                    "key": "PARKED_FIXTURE_PASSCODE",
                    "required": true,
                    "secret": true,
                    "description": "",
                }],
            })
            .to_string(),
        )
        .unwrap();
        claim::write_claim(
            &InstallClaim::new(
                "i-parked",
                name,
                "Parked Fixture",
                &tree,
                false,
                ClaimSource::LocalFile {
                    path: tmp.path().join("parkedfixture.brxt"),
                },
            )
            .parked(vec!["PARKED_FIXTURE_PASSCODE".to_string()]),
        )
        .unwrap();

        let err = handle_configure(name.to_string())
            .await
            .expect_err("a test process has no terminal, so this cannot prompt")
            .to_string();
        assert!(
            !err.contains("No extension named"),
            "the parked claim was not consulted: {err}"
        );
        assert!(err.contains("PARKED_FIXTURE_PASSCODE"), "{err}");
    }

    /// A local bundle that has since moved cannot be resumed, and the message
    /// has to name the file — "install failed" sends the user nowhere.
    #[test]
    fn a_resume_whose_bundle_is_gone_names_the_missing_file() {
        let err = resume_source(
            "spokeagent",
            ClaimSource::LocalFile {
                path: PathBuf::from("/definitely/not/here/spokeagent.brxt"),
            },
        )
        .expect_err("a missing bundle cannot be resumed")
        .to_string();
        assert!(
            err.contains("/definitely/not/here/spokeagent.brxt"),
            "{err}"
        );

        // A marketplace claim is re-fetchable, so it resumes.
        let source = resume_source(
            "spokeagent",
            ClaimSource::Marketplace {
                registry_id: "spokeagent-0.4.1".to_string(),
                url: "https://biorouter.ucsf.edu/x.brxt".to_string(),
            },
        )
        .unwrap();
        assert!(matches!(source, InstallSource::Marketplace { .. }));
    }
}
