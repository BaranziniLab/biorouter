//! Installing an extension as one transaction that can be parked, cancelled and
//! rolled back (#117).
//!
//! # Why a transaction and not a sequence of steps
//!
//! An extension install is seven things — download, validate, build the
//! environment, collect credentials, write them, register the config entry,
//! attach to the running chat — and the fourth **stops and waits for a person**.
//! Before this existed, each surface did its own subset in its own order and
//! none of them could wait: the CLI printed a warning about a missing required
//! value and registered the extension anyway, producing something that starts,
//! fails to authenticate, and reports success.
//!
//! Every failure path therefore has to undo the ones before it, or the machine
//! is left with a half-registered extension, an orphaned `~/.config/biorouter/
//! extensions/<name>/` tree, and a credential in the keychain for something that
//! is not installed. That undo is the reason these steps are one type.
//!
//! # Cancellation is a decision, not an error
//!
//! [`InstallState::Cancelled`] and [`InstallState::NeedsCredentials`] are
//! outcomes a caller reports, not failures it retries. An unattended run cannot
//! collect a passcode and must say so — with the key names, so the operator can
//! configure them and re-run — rather than installing something broken or
//! inventing a prompt nobody will see.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::agents::extension::{Envs, ExtensionConfig};
use crate::agents::extension_manager::ExtensionManager;
use crate::catalog::{
    CatalogChangeReason, CatalogEntryChange, CatalogEvents, CatalogExtensionChange,
    CatalogSkillChange,
};
use crate::config::extensions::{
    name_to_key, remove_extension, set_extension_silent, ExtensionEntry,
};
use crate::conversation::message::SecretDestination;
use crate::pending_user_action::UserActionOutcome;

use super::brxt::{
    extensions_root, run_uv_sync, secret_already_stored, uv_available, uv_missing_message,
    BrxtBundle, BrxtEnvVar, BrxtManifest, BundledSkill,
};
use super::claim::{self, ClaimSource, InstallClaim};
use super::credentials::{request_credentials, revoke, CredentialSpec, DEFAULT_CREDENTIAL_TTL};

/// Where the bundle comes from.
#[derive(Debug, Clone)]
pub enum InstallSource {
    /// A `.brxt` already on this machine: a drop, a Finder deep link, or
    /// `biorouter extension install <path>`.
    LocalFile { path: PathBuf },
    /// A BAAM marketplace entry. `registry_id` is issue #56 Task 43 (DR-23)
    /// provenance and is recorded beside the config entry, so the daemon
    /// re-derives the privacy tier from a stable id rather than from a config
    /// name anyone can rename.
    Marketplace { registry_id: String, url: String },
}

/// A terminal's echo-off prompt: handed the variables still to collect, returns
/// what the person typed.
///
/// A closure rather than a direct call so the prompt stays in the CLI crate,
/// where the terminal is, and this crate keeps no opinion about how a terminal
/// asks.
pub type TerminalPrompt =
    Box<dyn FnMut(&[BrxtEnvVar]) -> anyhow::Result<HashMap<String, String>> + Send>;

/// How this transaction may collect a value it does not have.
pub enum CredentialPolicy {
    /// Publish a credential card to `session_id` and park until a person
    /// answers it. The desktop and any agent-driven install.
    Ask {
        session_id: Option<String>,
        /// Groups parks that must die together — a bridged tool call passes the
        /// bridge nonce so the card dies with the turn.
        owner: Option<String>,
        ttl: Duration,
    },
    /// Ask on this process's terminal, with echo off. Interactive CLI only.
    Prompt(TerminalPrompt),
    /// Never ask. An unattended run stops at
    /// [`InstallState::NeedsCredentials`] and rolls back.
    Refuse,
}

impl std::fmt::Debug for CredentialPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ask { session_id, .. } => write!(f, "Ask({session_id:?})"),
            Self::Prompt(_) => write!(f, "Prompt"),
            Self::Refuse => write!(f, "Refuse"),
        }
    }
}

/// Where an install got to. Every variant is reportable to a user *and* to a
/// model, and none of them carries a value.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum InstallState {
    Downloading,
    Validating,
    /// Stopped because required values are missing and this run may not ask for
    /// them. `keys` are names, so an operator can configure them and re-run.
    NeedsCredentials {
        keys: Vec<String>,
    },
    Installing,
    /// Registered, and live in the session that asked for it.
    Attached,
    /// Registered, but not attached — no session asked, or the hot-load failed
    /// and the extension is still correct for the next chat.
    Installed,
    Cancelled,
    Failed {
        reason: String,
    },
}

impl InstallState {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Attached | Self::Installed)
    }
}

/// The result of a run, safe to hand to a model verbatim.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    /// Stable for the life of the transaction, so a retry can name it.
    pub install_id: String,
    pub state: InstallState,
    pub extension_name: Option<String>,
    pub display_name: Option<String>,
    /// **Names only.** There is nowhere in this struct a value can sit, and that
    /// is deliberate: this is what gets serialised into a tool result.
    pub configured_keys: Vec<String>,
    pub skills: Vec<BundledSkill>,
    pub enabled: bool,
    /// The operator had persisted `enabled: false` for this extension, so the
    /// package was updated and left switched off. Present so the caller can say
    /// WHY `enabled` is false, rather than leaving a model to guess that the
    /// install half-failed.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub operator_pinned_off: bool,
}

impl InstallReport {
    fn failed(install_id: String, reason: impl Into<String>) -> Self {
        Self {
            install_id,
            state: InstallState::Failed {
                reason: reason.into(),
            },
            extension_name: None,
            display_name: None,
            configured_keys: Vec::new(),
            skills: Vec::new(),
            enabled: false,
            operator_pinned_off: false,
        }
    }
}

/// One install, from a source to a registered (and possibly attached) extension.
pub struct ExtensionInstallTransaction {
    install_id: String,
    source: InstallSource,
    /// Values the caller already has — `--env` flags, a form the desktop filled
    /// in before the transaction started. Secret-declared keys among these are
    /// still written to the credential store, never to `config.yaml`.
    supplied: HashMap<String, String>,
    enable: bool,
    /// #42. The `enabled` flag `config.yaml` already carried for this
    /// extension, sampled **once, before this run writes anything**, and `None`
    /// when the file carried no entry the operator authored.
    ///
    /// ⚠ Sampling once is the whole point. Every later read of the config would
    /// see the row *this install just wrote*, so an install that registered
    /// `enabled: false` would go on to read its own write back as somebody
    /// else's decision — a pin it created itself. The value is filled in by
    /// `run_inner` before the credential phase; `None` until then, which is
    /// also the correct answer for a run that never reached it.
    operator_switch: Option<bool>,
    /// What this run actually persisted, or `None` when it registered nothing.
    /// Read only by [`ExtensionInstallTransaction::report`], so the report's
    /// `enabled` is the value on disk rather than a second guess at it.
    registered_enabled: Option<bool>,
    manager: Option<std::sync::Weak<ExtensionManager>>,
    /// Asked with the extension's REAL name — the one in the downloaded
    /// bundle's manifest — immediately before the attach. `Some(reason)` blocks
    /// the attach and is reported; the package is still installed.
    ///
    /// ⚠ The reason this exists at all: the caller's pre-flight can only check
    /// the name the REGISTRY advertised, and those are not the same string. A
    /// registry entry whose bundle declares a privacy-significant name would
    /// otherwise be attached on a pre-flight that never saw it.
    #[allow(clippy::type_complexity)]
    attach_guard: Option<Box<dyn Fn(&str) -> Option<String> + Send + Sync>>,
    /// Everything this run created, in the order it created it, so a failure
    /// undoes exactly its own work and nothing else.
    undo: Undo,
}

#[derive(Default)]
struct Undo {
    install_dir: Option<PathBuf>,
    /// Only true when THIS run extracted the tree. An install over an existing
    /// one must not delete the tree it found — a failed upgrade that removes a
    /// working extension is worse than a failed upgrade.
    created_install_dir: bool,
    config_key: Option<String>,
    /// Only keys this run wrote. A key the machine already held is left alone:
    /// deleting it would break every other extension sharing it.
    written_secrets: Vec<String>,
}

impl ExtensionInstallTransaction {
    pub fn new(source: InstallSource) -> Self {
        Self {
            install_id: uuid::Uuid::new_v4().to_string(),
            source,
            supplied: HashMap::new(),
            enable: true,
            operator_switch: None,
            registered_enabled: None,
            manager: None,
            attach_guard: None,
            undo: Undo::default(),
        }
    }

    /// Reuse an id from a stopped run so a resumed install keeps its identity.
    pub fn with_install_id(mut self, id: impl Into<String>) -> Self {
        self.install_id = id.into();
        self
    }

    pub fn with_values(mut self, values: HashMap<String, String>) -> Self {
        self.supplied = values;
        self
    }

    pub fn enabled(mut self, enable: bool) -> Self {
        self.enable = enable;
        self
    }

    /// The running session's extension manager, so a successful install can be
    /// hot-attached instead of waiting for a new chat.
    pub fn attach_to(mut self, manager: std::sync::Weak<ExtensionManager>) -> Self {
        self.manager = Some(manager);
        self
    }

    /// See [`ExtensionInstallTransaction::attach_guard`].
    pub fn guard_attach(
        mut self,
        guard: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.attach_guard = Some(Box::new(guard));
        self
    }

    pub fn install_id(&self) -> &str {
        &self.install_id
    }

    /// Run to completion, or to the first outcome that stops it.
    ///
    /// Never returns `Err`: every stop is an [`InstallState`] a caller can
    /// report. A caller that had to tell "refused" from "the machinery broke"
    /// would need a policy for the difference, and the safe policy is identical.
    pub async fn run(
        mut self,
        mut policy: CredentialPolicy,
        cancel: Option<&CancellationToken>,
    ) -> InstallReport {
        match self.run_inner(&mut policy, cancel).await {
            Ok(report) => report,
            Err(e) => {
                let reason = format!("{e:#}");
                self.rollback();
                InstallReport::failed(self.install_id.clone(), reason)
            }
        }
    }

    async fn run_inner(
        &mut self,
        policy: &mut CredentialPolicy,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<InstallReport> {
        if !uv_available() {
            anyhow::bail!("{}", uv_missing_message());
        }

        // ── download ──────────────────────────────────────────────────────
        let (bundle_path, _temp) = match &self.source {
            InstallSource::LocalFile { path } => (path.clone(), None),
            InstallSource::Marketplace { url, .. } => {
                let (path, guard) = download_bundle(url).await?;
                (path, Some(guard))
            }
        };

        // ── validate ──────────────────────────────────────────────────────
        let bundle = BrxtBundle::open(&bundle_path)?;
        let manifest = bundle.manifest().clone();
        let skills = bundle.skills().to_vec();
        let key = name_to_key(&manifest.name);
        // #42. Sampled HERE — the first point the extension's real name is
        // known, and still before this run has written a byte of config — so
        // the register step below decides against the operator's switch as it
        // stood when the install started, never against its own write.
        self.operator_switch = persisted_enabled_state(&manifest.name);

        // ── extract + build ───────────────────────────────────────────────
        let root = extensions_root();
        let install_dir = root.join(&manifest.name);
        // Belt and braces beside `BrxtBundle::open`'s name validation, and the
        // same shape `install_brxt_bundle` uses in the daemon. The BUNDLE names
        // this directory and `rollback` deletes it, so a name that escapes the
        // extensions root is a bundle that can delete an arbitrary tree.
        if !install_dir.starts_with(&root) {
            anyhow::bail!("Refusing to install under an invalid extension name");
        }
        let existed = install_dir.exists();
        let ctx = InstallContext {
            manifest: &manifest,
            skills: &skills,
            install_dir: &install_dir,
            existed_before: existed,
        };
        // Before the first byte is written, so a tree that appears on disk is
        // always a tree something on disk claims.
        self.record_claim(self.claim_for(&ctx));
        bundle.extract_to(&install_dir)?;
        self.undo.install_dir = Some(install_dir.clone());
        self.undo.created_install_dir = !existed;
        run_uv_sync(&install_dir)?;

        // ── credentials ───────────────────────────────────────────────────
        let mut values = ResolvedValues::default();
        if let Some(stopped) = self
            .settle_credentials(&ctx, &mut values, policy, cancel)
            .await?
        {
            return Ok(stopped);
        }
        let ResolvedValues { envs, mut env_keys } = values;

        // ── register ──────────────────────────────────────────────────────
        env_keys.sort();
        env_keys.dedup();
        let config = compose_config(&manifest, &install_dir, envs, env_keys.clone());
        // ⚠ **An install may update a package. It may not author the
        // operator's switch — in EITHER direction.** Issue #42's pin — a
        // persisted `enabled: false` — is enforced by `manage_extensions`,
        // which refuses without a proof-backed grant and refuses a private
        // extension outright. The install door consulted no entry at all, so
        // `enabled: self.enable` silently rewrote the pin machine-wide, for
        // every future chat, behind an approval card that says only "install".
        // Measured: `playwrightagent` went `false` -> `true` in config.yaml.
        //
        // The mirror of that is the same bug wearing the other sign, and it is
        // the one a `pinned_off` guard alone cannot catch: re-installing an
        // extension the user had switched ON, with `enable: false`, switched it
        // OFF machine-wide — and the row it wrote is then indistinguishable
        // from a deliberate operator pin, because
        // `privacy::refusal::extension_enable_refusal` reads exactly
        // "persisted AND `enabled: false`". So the install had, in one step,
        // disabled the extension and taken away the model's ability to undo it.
        //
        // [`enabled_to_persist`] is therefore the whole decision, and it is
        // asked against `self.operator_switch` — the flag sampled before this
        // run wrote anything. The real name is only knowable here, after the
        // bundle is read, which is why this cannot live at the call site.
        //
        // ⚠ The guard runs on the REAL name for the same reason. The caller's
        // pre-flight could only ask about the name the registry advertised, and
        // a registry that omits `extension_name` makes that the registry ID — a
        // different string, as SPOKEAgent's own entry proved
        // (`spokeagent-0.4.1` advertised, `spokeagent` installed).
        let guard_refusal = self
            .attach_guard
            .as_ref()
            .and_then(|guard| guard(&manifest.name));
        // What this run would ask for if the row were its to author: the
        // caller's flag, minus an attach the privacy guard refuses.
        let requested = self.enable && guard_refusal.is_none();
        let enable = enabled_to_persist(self.operator_switch, requested);
        // Attaching is a strictly smaller question than persisting: a run that
        // left an already-enabled extension alone must still not pull it into
        // *this* chat when the caller did not ask for it, or the guard refused.
        let attach_now = enable && requested;
        // Silent, then announced below with the bundle's skills folded in —
        // `set_extension` cannot see those, and two events for one install
        // would leave the second as the only complete one.
        set_extension_silent(ExtensionEntry {
            enabled: enable,
            config: config.clone(),
        });
        self.undo.config_key = Some(key.clone());
        self.registered_enabled = Some(enable);
        announce_install(&key, &manifest, &config, enable, &skills);
        if let InstallSource::Marketplace { registry_id, url } = &self.source {
            record_provenance(&manifest.name, registry_id, url, &install_dir);
        }

        // ── attach ────────────────────────────────────────────────────────
        let attached = attach_now && self.attach(config).await;
        // A claim that outlives a finished install is a permanent phantom
        // "pending install" on every reader.
        claim::remove_claim(&self.install_id);
        Ok(self.report(
            if attached {
                InstallState::Attached
            } else {
                InstallState::Installed
            },
            &manifest,
            &skills,
            &env_keys,
        ))
    }

    /// The credential phase: write what the caller already had, ask for what is
    /// missing, and re-check before anything is registered.
    ///
    /// `Ok(Some(report))` means the install **stopped here** — cancelled, or
    /// short of a required value with nobody to ask. Both leave a resume record
    /// and undo this run's config work; neither is an error.
    async fn settle_credentials(
        &mut self,
        ctx: &InstallContext<'_>,
        values: &mut ResolvedValues,
        policy: &mut CredentialPolicy,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<Option<InstallReport>> {
        let manifest = ctx.manifest;

        // `supplied` may already carry a declared-secret value (a `--secret`
        // flag, the desktop's own form). Those are written here rather than
        // being asked for again, and they go to the SAME place a card's answer
        // would: the credential store, with only the name recorded.
        self.apply_supplied(manifest, &mut values.envs, &mut values.env_keys)?;
        adopt_stored_secrets(manifest, &mut values.env_keys);

        let unmet: Vec<BrxtEnvVar> = manifest
            .env_vars
            .iter()
            .filter(|v| values.is_unmet(v))
            .cloned()
            .collect();
        let unmet_required: Vec<String> = unmet
            .iter()
            .filter(|v| v.required)
            .map(|v| v.key.clone())
            .collect();

        // ⚠ **A dialog is raised for what BLOCKS the install, never for what is
        // merely unset.** An `unmet` list is not a reason to interrupt a person:
        // SPOKEAgent declares an optional, non-secret `SPOKE_LOG_LEVEL`, and
        // asking `!unmet.is_empty()` parked a fully-satisfied install behind a
        // modal asking for a log level. Optional values still ride along on a
        // card that had to be shown anyway — they just cannot summon one.
        let blocks_install = unmet.iter().any(|v| v.required || v.secret);
        if blocks_install {
            match self
                .collect_credentials(manifest, &unmet, policy, cancel)
                .await?
            {
                Collected::Values {
                    configured_keys,
                    settings,
                } => {
                    values.envs.extend(settings);
                    for k in configured_keys {
                        if manifest.env_vars.iter().any(|v| v.key == k && v.secret)
                            && !values.env_keys.contains(&k)
                        {
                            values.env_keys.push(k.clone());
                            self.undo.written_secrets.push(k);
                        }
                    }
                }
                // ⚠ The SAME rule as the `Refused` arm below, and it has to be:
                // this arm used to stop unconditionally, so a person who
                // dismissed a dialog asking only for an OPTIONAL value threw
                // away an install whose every required value was already
                // satisfied. Declining to supply something optional is not a
                // decision to abandon the install.
                Collected::Cancelled if unmet_required.is_empty() => {
                    debug!(
                        "Installing {} without the optional values the user declined",
                        manifest.name
                    );
                }
                Collected::Cancelled => {
                    return Ok(Some(self.stop(ctx, &unmet, InstallState::Cancelled)));
                }
                Collected::Refused if unmet_required.is_empty() => {
                    // Only optional values are missing, and nobody can be
                    // asked. That is not a failure: the extension runs.
                    debug!("Installing {} without its optional values", manifest.name);
                }
                Collected::Refused => {
                    return Ok(Some(self.stop(
                        ctx,
                        &unmet,
                        InstallState::NeedsCredentials {
                            keys: unmet_required,
                        },
                    )));
                }
            }
        }

        // ⚠ The last gate before registration, and the reason this type exists.
        // A required value that is still missing here means the extension cannot
        // authenticate, and registering it anyway is what made an agent-driven
        // shell install report success for something permanently broken.
        let still_missing: Vec<String> = manifest
            .required_vars()
            .filter(|v| values.is_unmet(v))
            .map(|v| v.key.clone())
            .collect();
        if !still_missing.is_empty() {
            return Ok(Some(self.stop(
                ctx,
                &unmet,
                InstallState::NeedsCredentials {
                    keys: still_missing,
                },
            )));
        }
        Ok(None)
    }

    /// Stop before registration: keep the extracted tree, undo this run's
    /// config and credentials, and record what a retry still needs.
    fn stop(
        &mut self,
        ctx: &InstallContext<'_>,
        pending: &[BrxtEnvVar],
        state: InstallState,
    ) -> InstallReport {
        self.park_for_resume(ctx, pending);
        self.rollback_config_only();
        self.report(state, ctx.manifest, ctx.skills, &[])
    }

    /// Write the values the caller already had, splitting credentials from
    /// ordinary settings by what the manifest declared.
    fn apply_supplied(
        &mut self,
        manifest: &BrxtManifest,
        envs: &mut HashMap<String, String>,
        env_keys: &mut Vec<String>,
    ) -> anyhow::Result<()> {
        let (secret_keys, settings) = classify_supplied(manifest, &self.supplied);
        let config = crate::config::Config::global();
        for key in secret_keys {
            let value = &self.supplied[&key];
            config
                .set_secret(&key, value)
                .map_err(|e| anyhow::anyhow!("Failed to store `{key}`: {e}"))?;
            env_keys.push(key.clone());
            self.undo.written_secrets.push(key);
        }
        envs.extend(settings);
        Ok(())
    }

    async fn collect_credentials(
        &self,
        manifest: &BrxtManifest,
        unmet: &[BrxtEnvVar],
        policy: &mut CredentialPolicy,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<Collected> {
        match policy {
            CredentialPolicy::Refuse => Ok(Collected::Refused),
            CredentialPolicy::Prompt(ask) => {
                let values = ask(unmet)?;
                let mut configured_keys = Vec::new();
                let mut settings = HashMap::new();
                let config = crate::config::Config::global();
                for (key, value) in values {
                    if value.trim().is_empty() {
                        continue;
                    }
                    let secret = unmet.iter().any(|v| v.key == key && v.secret);
                    if secret {
                        config
                            .set_secret(&key, &value)
                            .map_err(|e| anyhow::anyhow!("Failed to store `{key}`: {e}"))?;
                    } else {
                        settings.insert(key.clone(), value);
                    }
                    configured_keys.push(key);
                }
                Ok(Collected::Values {
                    configured_keys,
                    settings,
                })
            }
            CredentialPolicy::Ask {
                session_id,
                owner,
                ttl,
            } => {
                let spec = CredentialSpec {
                    destination: SecretDestination::ExtensionEnv {
                        extension_name: manifest.name.clone(),
                    },
                    vars: unmet.to_vec(),
                };
                let prompt = format!(
                    "{} needs {} value{} before it can run.",
                    manifest.display_name,
                    unmet.len(),
                    if unmet.len() == 1 { "" } else { "s" }
                );
                let (outcome, settings) = request_credentials(
                    session_id.as_deref(),
                    owner.as_deref(),
                    prompt,
                    spec,
                    if ttl.is_zero() {
                        DEFAULT_CREDENTIAL_TTL
                    } else {
                        *ttl
                    },
                    cancel,
                )
                .await;
                match outcome {
                    UserActionOutcome::SecretsConfigured { configured_keys } => {
                        Ok(Collected::Values {
                            configured_keys,
                            settings,
                        })
                    }
                    UserActionOutcome::Cancelled | UserActionOutcome::TimedOut => {
                        Ok(Collected::Cancelled)
                    }
                    UserActionOutcome::Failed { reason } => Err(anyhow::anyhow!(reason)),
                    // The registry refuses a value-bearing answer to a secrets
                    // card, so these are unreachable by construction. Treating
                    // them as a refusal keeps the fail-safe direction if that
                    // ever changes.
                    other => {
                        warn!("Unexpected outcome for a credential card: {other:?}");
                        Ok(Collected::Cancelled)
                    }
                }
            }
        }
    }

    async fn attach(&self, config: ExtensionConfig) -> bool {
        if !self.enable {
            return false;
        }
        let Some(manager) = self.manager.as_ref().and_then(|m| m.upgrade()) else {
            return false;
        };
        match manager.add_extension(config).await {
            Ok(()) => true,
            Err(e) => {
                // Not a rollback. The config entry is correct and the next chat
                // will load it; only *this* chat missed out, and saying so is
                // more useful than undoing a good install.
                warn!("Installed extension could not be attached to the running chat: {e}");
                false
            }
        }
    }

    /// This run's claim, at the phase it starts in.
    fn claim_for(&self, ctx: &InstallContext<'_>) -> InstallClaim {
        InstallClaim::new(
            &self.install_id,
            &ctx.manifest.name,
            &ctx.manifest.display_name,
            ctx.install_dir,
            ctx.existed_before,
            ClaimSource::from(&self.source),
        )
    }

    /// Write the claim, and never fail an install over it.
    ///
    /// A claim that could not be written costs the *reclaim* path — the user
    /// has to reinstall rather than run `biorouter extension configure`.
    /// Aborting here would cost them the extension itself, which is worse.
    fn record_claim(&self, claim: InstallClaim) {
        if let Err(e) = claim::write_claim(&claim) {
            warn!(
                "Could not record the install claim for {}: {e}",
                claim.extension_name
            );
        }
    }

    /// Rewrite this run's claim to say it stopped, and on which key names.
    ///
    /// ⚠ **The claim is the only record that outlives the process.** What this
    /// replaced was a process-global map holding a `bundle_path` that, for a
    /// marketplace install, pointed into a `TempDir` already dropped — so it
    /// was unreadable after a restart and dangling before one. The claim
    /// records the re-fetchable source instead, and key NAMES only.
    fn park_for_resume(&self, ctx: &InstallContext<'_>, pending: &[BrxtEnvVar]) {
        let keys = pending.iter().map(|v| v.key.clone()).collect();
        self.record_claim(self.claim_for(ctx).parked(keys));
    }

    /// Undo the registration and the credentials, but keep the extracted tree
    /// and its built environment.
    ///
    /// The tree is the expensive half and contains nothing sensitive, and
    /// `run_uv_sync` is incremental against the surviving `.venv`. A resumed
    /// install *does* re-download and re-extract — `run_inner` extracts
    /// unconditionally — it just does not pay for the environment again.
    fn rollback_config_only(&mut self) {
        if let Some(key) = self.undo.config_key.take() {
            remove_extension(&key);
        }
        revoke(&std::mem::take(&mut self.undo.written_secrets));
    }

    /// Undo everything this run created.
    fn rollback(&mut self) {
        self.rollback_config_only();
        if self.undo.created_install_dir {
            if let Some(dir) = self.undo.install_dir.take() {
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
        claim::remove_claim(&self.install_id);
    }

    fn report(
        &self,
        state: InstallState,
        manifest: &BrxtManifest,
        skills: &[BundledSkill],
        configured_keys: &[String],
    ) -> InstallReport {
        // #42, read off the ONE pre-write sample rather than off the config
        // file: after `set_extension_silent` the file carries this run's own
        // row, so re-reading it here would report an install's own
        // `enable: false` back to the caller as "the operator pinned it".
        let pinned_off = self.enable && self.operator_switch == Some(false);
        InstallReport {
            install_id: self.install_id.clone(),
            // What is on disk, not a second derivation of it. A run that
            // registered nothing (cancelled, or short of a credential) reports
            // `false`, which is what `state.is_success()` already said.
            enabled: state.is_success() && self.registered_enabled.unwrap_or(false),
            state,
            extension_name: Some(manifest.name.clone()),
            display_name: Some(manifest.display_name.clone()),
            configured_keys: configured_keys.to_vec(),
            skills: skills.to_vec(),
            operator_pinned_off: pinned_off,
        }
    }
}

/// Split caller-supplied values into credentials and ordinary settings.
///
/// **Public because this is the decision that decides what may be written to
/// `config.yaml`**, and a decision that important should be assertable without
/// running an install. Returns key names for the credential half and full
/// values for the settings half — the caller writes the credentials to the
/// credential store from `supplied`, which is the only map that holds them.
///
/// A key the manifest never declared is an ad-hoc **setting**, not a credential.
/// Guessing the other way would hide a value the user expected to see in their
/// own config, and `--secret KEY=VALUE` already exists for the case where they
/// meant a credential.
pub fn classify_supplied(
    manifest: &BrxtManifest,
    supplied: &HashMap<String, String>,
) -> (Vec<String>, HashMap<String, String>) {
    let mut secret_keys = Vec::new();
    let mut settings = HashMap::new();
    for (key, value) in supplied {
        if value.trim().is_empty() {
            continue;
        }
        let declared_secret = manifest
            .env_vars
            .iter()
            .find(|v| &v.key == key)
            .map(|v| v.secret)
            .unwrap_or(false);
        if declared_secret {
            secret_keys.push(key.clone());
        } else {
            settings.insert(key.clone(), value.clone());
        }
    }
    secret_keys.sort();
    (secret_keys, settings)
}

/// The config entry an install registers.
///
/// **Public for the same reason as [`classify_supplied`]**: `config.yaml` is a
/// plaintext file on disk, this is the only function that decides what goes into
/// it, and "no credential is ever in `envs`" is a property a test must be able
/// to state directly rather than infer from a successful install.
pub fn compose_config(
    manifest: &BrxtManifest,
    install_dir: &std::path::Path,
    envs: HashMap<String, String>,
    mut env_keys: Vec<String>,
) -> ExtensionConfig {
    env_keys.sort();
    env_keys.dedup();
    ExtensionConfig::Stdio {
        name: manifest.name.clone(),
        description: manifest.description.clone(),
        cmd: "uv".to_string(),
        args: vec![
            "run".to_string(),
            "--directory".to_string(),
            install_dir.display().to_string(),
            manifest.entry_point.clone(),
        ],
        envs: Envs::new(envs),
        env_keys,
        timeout: Some(300),
        bundled: None,
        available_tools: Vec::new(),
    }
}

/// Record the declared secrets the machine ALREADY holds, so the spawner is
/// told to inject them.
///
/// ⚠ **A secret being "met" and a secret being RECORDED are two different
/// things, and conflating them shipped a broken install.** `is_unmet` counts a
/// value already in the credential store as satisfied — correctly, because
/// re-asking for a passcode the machine holds trains the user to paste ones
/// they need not. But `env_keys` is the *only* thing that tells the spawner to
/// pull a secret back out of the store and put it in the child's environment,
/// and it was previously appended to only for keys this run wrote. So a
/// reinstall of an extension whose passcode was already stored produced
/// `env_keys: []`: registered, `enabled: true`, and permanently unable to
/// start, while the install reported success. Reproduced with SPOKEAgent —
/// `RuntimeError: SPOKEAGENT_PASSCODE environment variable is required`.
///
/// Adopted keys deliberately do NOT join `undo.written_secrets`: this run did
/// not write them, they may be shared with another extension, and a rollback
/// that revoked them would break whatever else depends on them.
/// Takes the store predicate as an argument so the RULE is testable without a
/// real credential store — the alternative is a test that can only run on a
/// machine that already holds the secret, which is how this went unnoticed.
fn adopt_stored_secrets_with(
    manifest: &BrxtManifest,
    env_keys: &mut Vec<String>,
    is_stored: impl Fn(&str) -> bool,
) {
    for var in &manifest.env_vars {
        if var.secret && !env_keys.iter().any(|k| k == &var.key) && is_stored(&var.key) {
            env_keys.push(var.key.clone());
        }
    }
}

fn adopt_stored_secrets(manifest: &BrxtManifest, env_keys: &mut Vec<String>) {
    adopt_stored_secrets_with(manifest, env_keys, secret_already_stored);
}

/// The `enabled` flag the operator's own `config.yaml` carries for this
/// extension, or `None` when the file has no entry for it.
///
/// Issue #42's pin, generalised from "is it pinned off?" to "what does the
/// switch say?", because an install has to honour it in both positions.
///
/// "Persisted" is the load-bearing half: a default-off PLATFORM extension is
/// absent from the config file and injected with its default by
/// [`get_extension_entry_by_name`](crate::config::extensions::get_extension_entry_by_name),
/// so `extension_entry_is_persisted` is what separates a deliberate operator
/// decision from an injected default. An injected default answers `None` here
/// and the install authors the row, exactly as it does for a first install.
fn persisted_enabled_state(extension_name: &str) -> Option<bool> {
    if !crate::config::extensions::extension_entry_is_persisted(extension_name) {
        return None;
    }
    crate::config::extensions::get_extension_entry_by_name(extension_name)
        .map(|entry| entry.enabled)
}

/// What an install may write to an extension's `enabled` flag.
///
/// `operator_switch` is [`persisted_enabled_state`] sampled **before** this run
/// wrote anything; `None` means the config file carried no entry, so this
/// install is the row's author and the caller's request stands.
///
/// ⚠ **An install updates a PACKAGE. It never authors a switch somebody else
/// already set.** Both directions are real defects, and a guard that only
/// looked at one of them is what shipped:
///
///  * `Some(false)` + `requested: true` — #42's pin. `manage_extensions`
///    refuses to re-enable a persisted `enabled: false` without a proof-backed
///    grant, and refuses a private extension outright; an install that flipped
///    it to `true` overturned that behind a card saying only "install".
///  * `Some(true)` + `requested: false` — the mirror. Re-installing (an
///    upgrade, a marketplace re-install, a resumed run) with `enable: false`
///    switched OFF an extension the user was using, machine-wide, and left
///    behind a row `privacy::refusal::extension_enable_refusal` reads as an
///    operator pin — so the model could not put it back either.
fn enabled_to_persist(operator_switch: Option<bool>, requested: bool) -> bool {
    operator_switch.unwrap_or(requested)
}

/// What the credential phase is working on, so the helpers below take one
/// borrow instead of four.
struct InstallContext<'a> {
    manifest: &'a BrxtManifest,
    skills: &'a [BundledSkill],
    install_dir: &'a Path,
    /// Whether the tree was already there when this run started. Sampled before
    /// anything can create it, and recorded on the claim so a reader can tell a
    /// first install from an upgrade that parked.
    existed_before: bool,
}

/// Values resolved so far: settings bound for `config.yaml`, and the NAMES of
/// credentials written to the credential store.
#[derive(Default)]
struct ResolvedValues {
    envs: HashMap<String, String>,
    env_keys: Vec<String>,
}

impl ResolvedValues {
    /// Whether the variable still has to come from somewhere. A secret already in the
    /// credential store counts as met — an install that re-asks for a passcode
    /// the machine already holds trains the user to paste ones they need not.
    fn is_unmet(&self, var: &BrxtEnvVar) -> bool {
        !(self.envs.contains_key(&var.key)
            || (var.secret && self.env_keys.iter().any(|k| k == &var.key))
            || var.has_stored_secret())
    }
}

enum Collected {
    Values {
        configured_keys: Vec<String>,
        settings: HashMap<String, String>,
    },
    Cancelled,
    Refused,
}

/// Issue #112. Announce the finished install, with the skills its bundle
/// carried.
///
/// `bundled_skill_ids` is what Worktree 5's skill inventory keys off. It states
/// what the BUNDLE contains — not what has been installed to the skills
/// directory, which this path does not do — so a consumer treats it as "look
/// here", never as "these are present".
fn announce_install(
    key: &str,
    manifest: &BrxtManifest,
    config: &ExtensionConfig,
    enabled: bool,
    skills: &[BundledSkill],
) {
    let skill_ids: Vec<String> = skills.iter().map(|s| s.slug.clone()).collect();
    let row = CatalogExtensionChange {
        key: key.to_string(),
        name: manifest.name.clone(),
        display_name: Some(manifest.display_name.clone()),
        change: CatalogEntryChange::Added,
        config: Some(config.clone()),
        enabled,
        bundled_skill_ids: skill_ids,
    };
    let skill_rows: Vec<CatalogSkillChange> = skills
        .iter()
        .map(|skill| CatalogSkillChange {
            id: skill.slug.clone(),
            name: Some(skill.name.clone()),
            change: CatalogEntryChange::Added,
            source_extension_key: Some(key.to_string()),
        })
        .collect();
    CatalogEvents::global().publish(CatalogChangeReason::Install, vec![row], skill_rows, None);
}

/// Issue #56 Task 43 (DR-23). Record where a marketplace bundle came from, so
/// the privacy tier is re-derived from a stable registry id rather than from a
/// config name the user (or the model) can rename.
///
/// `install_dir` is what survives that rename — the `--directory` argument
/// cannot move without breaking the launch — so it is recorded alongside.
fn record_provenance(name: &str, registry_id: &str, url: &str, install_dir: &std::path::Path) {
    let provenance = crate::privacy::provenance::ExtensionProvenance {
        install_id: None,
        registry_id: registry_id.to_string(),
        install_dir: Some(install_dir.display().to_string()),
        source_url: Some(url.to_string()),
        bundle_sha256: None,
        recorded_at: Some(chrono::Utc::now().to_rfc3339()),
    };
    if let Err(e) = crate::privacy::provenance::record(name, provenance) {
        // A missing provenance record fails CLOSED at read time — the tier
        // falls back to the config-name join — so this is a degradation, not a
        // reason to undo a good install.
        warn!("Could not record marketplace provenance for {name}: {e}");
    }
}

/// A downloaded bundle, and the temp dir that owns it.
async fn download_bundle(url: &str) -> anyhow::Result<(PathBuf, tempfile::TempDir)> {
    let parsed = url::Url::parse(url).map_err(|_| anyhow::anyhow!("Not a valid URL: {url}"))?;
    if parsed.scheme() != "https" {
        anyhow::bail!("Refusing to download an extension over {}", parsed.scheme());
    }
    let response = reqwest::get(url)
        .await
        .map_err(|e| anyhow::anyhow!("Could not download the bundle: {e}"))?;
    if !response.status().is_success() {
        anyhow::bail!("Could not download the bundle: HTTP {}", response.status());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| anyhow::anyhow!("Could not read the bundle: {e}"))?;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("bundle.brxt");
    std::fs::write(&path, &bytes)?;
    Ok((path, dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The report is what a tool result carries back to the model. Nothing in
    /// its shape can hold a value, and this asserts that against a state that
    /// names every key it is still waiting for.
    #[test]
    fn a_report_serialises_key_names_and_never_a_value() {
        let report = InstallReport {
            install_id: "i-1".to_string(),
            state: InstallState::NeedsCredentials {
                keys: vec!["SPOKEAGENT_PASSCODE".to_string()],
            },
            extension_name: Some("spokeagent".to_string()),
            display_name: Some("SPOKE Agent".to_string()),
            configured_keys: vec!["SPOKEAGENT_PASSCODE".to_string()],
            skills: Vec::new(),
            enabled: false,
            operator_pinned_off: false,
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("SPOKEAGENT_PASSCODE"));
        assert!(json.contains("needsCredentials"));
        assert!(!json.contains("value"));
    }

    #[test]
    fn only_a_registered_or_attached_install_counts_as_success() {
        assert!(InstallState::Attached.is_success());
        assert!(InstallState::Installed.is_success());
        assert!(!InstallState::Cancelled.is_success());
        assert!(!InstallState::NeedsCredentials { keys: vec![] }.is_success());
        assert!(!InstallState::Failed {
            reason: "x".to_string()
        }
        .is_success());
    }

    #[tokio::test]
    async fn an_http_bundle_url_is_refused_before_any_request_is_made() {
        let err = download_bundle("http://example.test/x.brxt")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Refusing to download"));
    }

    fn spoke_manifest() -> BrxtManifest {
        BrxtManifest {
            name: "spokeagent".to_string(),
            display_name: "SPOKEAgent".to_string(),
            description: "fixture".to_string(),
            version: "0.4.1".to_string(),
            entry_point: "spokeagent".to_string(),
            repository: "https://github.com/BaranziniLab/SPOKEAgent".to_string(),
            tools_count: None,
            env_vars: vec![
                BrxtEnvVar {
                    key: "SPOKEAGENT_PASSCODE".to_string(),
                    required: true,
                    auto_propagate: false,
                    default: None,
                    description: "Access passcode provided by UCSF".to_string(),
                    secret: true,
                },
                BrxtEnvVar {
                    key: "SPOKE_LOG_LEVEL".to_string(),
                    required: false,
                    auto_propagate: false,
                    default: None,
                    description: "Logging level".to_string(),
                    secret: false,
                },
            ],
        }
    }

    /// The reinstall bug, as a rule rather than as a machine state.
    ///
    /// `env_keys` is the ONLY thing that tells the spawner to pull a secret out
    /// of the store for the child, and it used to be appended to only for keys
    /// the run itself wrote. A secret already stored is "met", so it was never
    /// collected and never recorded — producing `env_keys: []` on every
    /// reinstall, and a registered extension that could not start.
    #[test]
    fn a_secret_the_machine_already_holds_is_still_recorded_on_the_config() {
        let manifest = spoke_manifest();
        let mut env_keys = Vec::new();
        adopt_stored_secrets_with(&manifest, &mut env_keys, |key| key == "SPOKEAGENT_PASSCODE");
        assert_eq!(env_keys, vec!["SPOKEAGENT_PASSCODE".to_string()]);
    }

    /// Three things adoption must NOT do: invent a key the store does not hold,
    /// promote a non-secret setting into the credential list (those belong in
    /// `envs`), or duplicate one this run already wrote.
    #[test]
    fn adoption_never_invents_duplicates_or_promotes_a_plain_setting() {
        let manifest = spoke_manifest();

        let mut none_stored = Vec::new();
        adopt_stored_secrets_with(&manifest, &mut none_stored, |_| false);
        assert!(none_stored.is_empty());

        // `SPOKE_LOG_LEVEL` is not a secret; even a store that claims to hold
        // every key must not move it out of `envs`.
        let mut everything_stored = Vec::new();
        adopt_stored_secrets_with(&manifest, &mut everything_stored, |_| true);
        assert_eq!(
            everything_stored,
            vec!["SPOKEAGENT_PASSCODE".to_string()],
            "a non-secret setting must never be recorded as a credential"
        );

        let mut already_written = vec!["SPOKEAGENT_PASSCODE".to_string()];
        adopt_stored_secrets_with(&manifest, &mut already_written, |_| true);
        assert_eq!(already_written.len(), 1);
    }

    #[tokio::test]
    async fn stored_secret_satisfies_only_a_secret_variable() {
        let secret = BrxtEnvVar {
            key: "SHARED_NAME".to_string(),
            required: true,
            auto_propagate: false,
            default: None,
            description: String::new(),
            secret: true,
        };
        let ordinary = BrxtEnvVar {
            secret: false,
            ..secret.clone()
        };
        let values = ResolvedValues::default();

        crate::config::with_config_overrides(
            HashMap::from([(String::from("SHARED_NAME"), String::from("stored-secret"))]),
            async {
                assert!(!values.is_unmet(&secret));
                assert!(values.is_unmet(&ordinary));

                let values_with_secret_key = ResolvedValues {
                    env_keys: vec!["SHARED_NAME".to_string()],
                    ..ResolvedValues::default()
                };
                assert!(values_with_secret_key.is_unmet(&ordinary));

                let values_with_setting = ResolvedValues {
                    envs: HashMap::from([(
                        "SHARED_NAME".to_string(),
                        "ordinary-setting".to_string(),
                    )]),
                    ..ResolvedValues::default()
                };
                assert!(!values_with_setting.is_unmet(&ordinary));
            },
        )
        .await;
    }

    /// Issue #42's operator switch, on the install door — in **both**
    /// positions.
    ///
    /// `manage_extensions` refuses a persisted `enabled: false` without a
    /// proof-backed grant, and refuses a private extension outright. The install
    /// door consulted no config entry at all, so it rewrote the pin machine-wide
    /// behind a card that says only "install" — measured, `playwrightagent` went
    /// false -> true in config.yaml.
    ///
    /// The mirror row is the one the original guard could not catch, and it is
    /// the more damaging of the two: an install that switches OFF an extension
    /// the user had on does not merely disable it, it leaves behind exactly the
    /// row `privacy::refusal::extension_enable_refusal` reads as an operator
    /// pin — so the model is then refused when it tries to put it back, and for
    /// a PRIVATE extension the proof-backed escape hatch does not apply either.
    ///
    /// ⚠ Fails an implementation that returns `requested` (the shipped one),
    /// and equally one that returns `operator_switch.unwrap_or(false)` or
    /// `unwrap_or(true)` — a first install has to be able to land in either
    /// position.
    #[test]
    fn an_install_may_update_a_package_but_never_authors_the_operators_switch() {
        assert!(
            enabled_to_persist(Some(true), false),
            "re-installing with `enable: false` switched off an extension the user had ON, and \
             the row it wrote then reads as an operator pin the model may not undo"
        );
        assert!(
            !enabled_to_persist(Some(false), true),
            "#42: an install must not overturn a persisted `enabled: false`"
        );
        // No persisted entry: this install authors the row, so the caller's
        // request stands. `--no-enable` and the model's `enable: false` both
        // depend on the second row here.
        assert!(enabled_to_persist(None, true));
        assert!(!enabled_to_persist(None, false));
    }

    /// The claim is a plaintext file in the user's config directory. Same rule
    /// as the report: key NAMES, and no shape a value fits in.
    ///
    /// The assertion is structural rather than "this particular secret is
    /// absent", because the failure it guards against is somebody adding a
    /// `supplied` map or a resolved env map to `InstallClaim` to make resuming
    /// easier — which no value-specific test would ever fail on.
    #[test]
    fn a_parked_claim_records_key_names_and_never_a_value() {
        let manifest = spoke_manifest();
        let claim = InstallClaim::new(
            "i-1",
            &manifest.name,
            &manifest.display_name,
            Path::new("/ext/spokeagent"),
            false,
            ClaimSource::LocalFile {
                path: PathBuf::from("/bundles/spokeagent.brxt"),
            },
        )
        .parked(vec!["SPOKEAGENT_PASSCODE".to_string()]);

        let json = serde_json::to_string(&claim).unwrap();
        assert!(json.contains("SPOKEAGENT_PASSCODE"), "{json}");
        assert!(json.contains("\"parked\""), "{json}");
        for field in ["\"value\"", "\"values\"", "\"supplied\"", "\"envs\""] {
            assert!(
                !json.contains(field),
                "a claim grew a field a credential fits in: {json}"
            );
        }
    }

    /// ⚠ The literal shape the in-memory record had, and the reason it could
    /// never be resumed: `bundle_path` for a marketplace install pointed inside
    /// a `TempDir` dropped when the install returned. A claim records the URL,
    /// which is the half a resume can act on.
    #[test]
    fn a_marketplace_claim_records_the_url_not_a_temp_bundle_path() {
        let manifest = spoke_manifest();
        let claim = InstallClaim::new(
            "i-2",
            &manifest.name,
            &manifest.display_name,
            Path::new("/ext/spokeagent"),
            false,
            ClaimSource::Marketplace {
                registry_id: "spokeagent-0.4.1".to_string(),
                url: "https://biorouter.ucsf.edu/bundles/spokeagent.brxt".to_string(),
            },
        )
        .parked(vec!["SPOKEAGENT_PASSCODE".to_string()]);

        let json = serde_json::to_string(&claim).unwrap();
        let temp = std::env::temp_dir();
        let temp = temp.to_string_lossy();
        assert!(
            !json.contains(temp.as_ref()),
            "the claim recorded a path under the download's temp dir ({temp}): {json}"
        );
        assert!(json.contains("https://biorouter.ucsf.edu/"), "{json}");
        assert!(json.contains("spokeagent-0.4.1"), "{json}");
    }

    /// An optional, non-secret variable must not be able to summon a dialog —
    /// the predicate the credential phase now branches on.
    #[test]
    fn only_a_required_or_secret_value_can_raise_a_credential_dialog() {
        let manifest = spoke_manifest();
        let optional_only = [manifest.env_vars[1].clone()];
        assert!(
            !optional_only.iter().any(|v| v.required || v.secret),
            "SPOKE_LOG_LEVEL alone must not block an install"
        );
        assert!(
            manifest.env_vars.iter().any(|v| v.required || v.secret),
            "the passcode must still be able to raise one"
        );
    }
}
