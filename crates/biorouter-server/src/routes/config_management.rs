use crate::routes::utils::{check_provider_configured, provider_readiness, ProviderReadiness};
use crate::state::AppState;
use axum::routing::put;
use axum::{
    extract::Path,
    routing::{delete, get, post},
    Json, Router,
};
use biorouter::config::declarative_providers::LoadedProvider;
use biorouter::config::extension_credentials::ExtensionCredential;
use biorouter::config::paths::Paths;
use biorouter::config::ExtensionEntry;
use biorouter::config::{Config, ConfigError, ConfigWriteFailure};
use biorouter::model::ModelConfig;
use biorouter::privacy::ProviderTier;
use biorouter::providers::auto_detect::{detect_provider_from_api_key, detectable_providers};
use biorouter::providers::base::{ConfigKey, ProviderAffiliation, ProviderMetadata, ProviderType};
use biorouter::providers::create_with_default_model;
use biorouter::providers::errors::ProviderError;
use biorouter::providers::pricing::{resolved_provider_model_pricing, ProviderModelPricing};
use biorouter::providers::providers as get_providers;
use biorouter::providers::{retry_operation, RetryConfig};
use biorouter::{
    agents::execute_commands, agents::ExtensionConfig, config::permission::PermissionLevel,
    privacy::PrivacyRefusal, slash_commands,
};
// Issue #56 DR-16. The LIB path, not `crate::auth` — see the note on the same
// import in `routes::agent`.
use biorouter_server::auth::is_user_action;
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_yaml;
use std::{collections::HashMap, sync::Arc};
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct ExtensionResponse {
    pub extensions: Vec<ExtensionEntry>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct ExtensionQuery {
    pub name: String,
    pub config: ExtensionConfig,
    pub enabled: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct UpsertConfigQuery {
    pub key: String,
    pub value: Value,
    pub is_secret: bool,
    /// Issue #56 Task 30. The typed confirmation Settings → Privacy sends with a
    /// write to `BIOROUTER_PRIVACY_TIERS`, and nothing else sends at all.
    ///
    /// ⚠ **What this is and what it is not.** It is a **UX guard against an
    /// accidental or model-composed config write**, not an authorization
    /// boundary: the phrase is a fixed string in the shipped source, so a caller
    /// holding the daemon secret replays it. That is acceptable because
    /// `check_token` has no principal — the daemon cannot tell Settings →
    /// Privacy from any other loopback caller — and because the *authorization*
    /// on this route is `X-User-Action`, not the phrase. What the phrase buys is
    /// that the flip cannot be a side effect of an ordinary `/config/upsert`,
    /// which is the reachable path: a model *can* compose one of those through a
    /// tool.
    ///
    /// ⚠ **This comment used to justify the phrase by adding *"and a caller that
    /// already holds the secret can raise its own session to private capability
    /// anyway"*, citing AR-15. That is no longer true and is withdrawn** —
    /// AR-15 was retired on 2026-08-02 by DR-16 (commit `0757823f`), which made
    /// an upward provider bind require `X-User-Action`. Do not restore the
    /// argument: the guard does not need it, and a stale "we are already open
    /// here anyway" is how a weakened control gets waved through.
    #[serde(default)]
    pub confirm: Option<String>,
}

#[derive(Deserialize, Serialize, ToSchema)]
pub struct ConfigKeyQuery {
    pub key: String,
    pub is_secret: bool,
}

#[derive(Serialize, ToSchema)]
pub struct ConfigResponse {
    pub config: HashMap<String, Value>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProviderDetails {
    pub name: String,
    pub metadata: ProviderMetadata,
    pub is_configured: bool,
    pub provider_type: ProviderType,
    /// DR-26's third axis for this provider, resolved from a live **instance**
    /// (issue #56). `None` = a public provider, which has no affiliation at all,
    /// or one this daemon could not resolve — see [`resolve_provider_affiliation`].
    ///
    /// ⚠ **Here rather than on [`ProviderMetadata`], deliberately.** That struct
    /// is documented top to bottom as the *type-level* claim — its own `tier`
    /// field carries the warning "do not hang a badge on this field", because a
    /// re-pointed `ollama` still ships `Private` there while its instance
    /// resolves Public. Affiliation is the opposite kind of value: DR-26 requires
    /// it come off the instance, so that a Versa module repointed elsewhere loses
    /// Private and `ucsf` together. Putting an instance-resolved field inside a
    /// type-level struct is how the next reader comes to believe the tier beside
    /// it is instance-resolved too.
    #[serde(default)]
    pub affiliation: Option<ProviderAffiliation>,
    /// The tier of a live **instance** of this provider — the value Gate C
    /// actually judges a tool call on (issue #56).
    ///
    /// ⚠ **This is not `metadata.tier`, and the difference is a user-visible
    /// bug it exists to close.** `metadata.tier` is the type-level claim: what
    /// the module ships. This is what `providers::create` resolved *here*, off
    /// the same instance as [`Self::affiliation`] and in the same call, so the
    /// two axes can never be sampled at different instants or off different
    /// endpoints. A re-pointed `ollama` ships `private` in its metadata and
    /// resolves `public` here; a Versa module re-pointed off the UCSF gateway
    /// loses Private and `ucsf` together, in this field and the one above.
    ///
    /// ⚠ **`None` means "not resolved", NEVER "public".** It is the answer for
    /// an unconfigured provider, a construction failure and a timeout alike, and
    /// every consumer must treat it as *judge nothing* rather than as the
    /// permissive tier. Collapsing it to Public is what made a Private UCSF
    /// model read as public on the composer's extension menu; the renderer's
    /// `extensionPairingRefused` documents the same rule on its side.
    #[serde(default)]
    pub resolved_tier: Option<ProviderTier>,
    /// Why a provider the user HAS set up cannot run right now: a one-line
    /// sentence for the model picker to print on the row it disables.
    ///
    /// Set only when [`Self::is_configured`] is false for a reason other than a
    /// missing key — today, a coding agent whose command key is saved and whose
    /// CLI does not resolve (see `routes::utils::provider_readiness`). `None` for
    /// every usable provider and for every provider that is simply not set up,
    /// which the picker leaves out rather than greys out.
    ///
    /// ⚠ **Only what can be learned without spawning.** A signed-out CLI is not
    /// reported here: finding that out means running it, and this route runs
    /// for every provider on every settings open.
    #[serde(default)]
    pub unavailable_reason: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ProvidersResponse {
    pub providers: Vec<ProviderDetails>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ToolPermission {
    pub tool_name: String,
    pub permission: PermissionLevel,
}

#[derive(Deserialize, ToSchema)]
pub struct UpsertPermissionsQuery {
    pub tool_permissions: Vec<ToolPermission>,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateCustomProviderRequest {
    pub engine: String,
    pub display_name: String,
    pub api_url: String,
    pub api_key: String,
    pub models: Vec<String>,
    pub supports_streaming: Option<bool>,
    pub headers: Option<std::collections::HashMap<String, String>>,
}

#[derive(Deserialize, ToSchema)]
pub struct CheckProviderRequest {
    pub provider: String,
    /// W2-PRV-2. Also make one cheap authenticated call (listing the provider's
    /// models, bounded at [`LIVE_CHECK_TIMEOUT`]) and refuse the check when the
    /// provider rejects the credentials. Constructing a provider makes no
    /// network call, so without this a wrong key passed. A provider with no
    /// secret, or with no live model listing, is checked as before.
    #[serde(default)]
    pub live: bool,
    /// W2-PRV-2. Values to check BEFORE they are saved: keys this provider
    /// declares, applied as task-local overrides for the check only
    /// (`with_config_overrides`, the mechanism provider auto-detection uses).
    /// Nothing is written, so a rejected key never replaces a working one.
    #[serde(default)]
    pub candidate: Option<HashMap<String, String>>,
}

/// How long [`check_provider`]'s live call may take. A check that cannot finish
/// in time says nothing about the credentials, so it passes, as it did before.
const LIVE_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Deserialize, ToSchema)]
pub struct SetProviderRequest {
    pub provider: String,
    pub model: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MaskedSecret {
    pub masked_value: String,
}

#[derive(Serialize, ToSchema)]
#[serde(untagged)]
pub enum ConfigValueResponse {
    Value(Value),
    MaskedValue(MaskedSecret),
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub enum CommandType {
    Builtin,
    Workflow,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SlashCommand {
    pub command: String,
    pub help: String,
    pub command_type: CommandType,
}
#[derive(Serialize, ToSchema)]
pub struct SlashCommandsResponse {
    pub commands: Vec<SlashCommand>,
}

#[derive(Deserialize, ToSchema)]
pub struct DetectProviderRequest {
    pub api_key: String,
}

#[derive(Serialize, ToSchema)]
pub struct DetectProviderResponse {
    /// The detected provider, or `null` when detection failed.
    pub provider_name: Option<String>,
    /// Exact secret config key used by the detected provider.
    pub api_key_config_key: Option<String>,
    /// All model ids the provider reported for the key (empty on failure).
    #[serde(default)]
    pub models: Vec<String>,
    /// A recommended default chat model, when one could be determined.
    pub default_model: Option<String>,
    /// Non-secret config to persist alongside the key (e.g. a regional host).
    #[serde(default)]
    pub extra_config: HashMap<String, String>,
    /// Machine-readable failure reason when `provider_name` is null:
    /// `"timeout" | "network" | "invalid_key" | "no_match"`.
    pub reason: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct DetectableProvider {
    pub name: String,
    pub display_name: String,
}

#[derive(Serialize, ToSchema)]
pub struct DetectableProvidersResponse {
    pub providers: Vec<DetectableProvider>,
}
#[utoipa::path(
    post,
    path = "/config/upsert",
    request_body = UpsertConfigQuery,
    responses(
        (status = 200, description = "Configuration value upserted successfully", body = String),
        (status = 400, description = "Refused (issue #56, DR-27): \
                                      `BIOROUTER_PRIVACY_MIXING_POLICY` is one of 'open', \
                                      'standard' or 'strict'. Also `BIOROUTER_MAX_TURNS`, which \
                                      must be a whole number of at least 1"),
        (status = 403, description = "Refused: `BIOROUTER_PRIVACY_TIERS` is the master privacy \
                                      switch and may only be written from Settings > App > \
                                      Privacy, \
                                      with its typed confirmation, or (issue #56, DR-27) \
                                      relaxing `BIOROUTER_PRIVACY_MIXING_POLICY` needed a system \
                                      authentication that did not happen"),
        (status = 409, description = "Refused by a privacy boundary (issue #56, DR-16): the key \
                                      decides what privacy capability new chats start at, so \
                                      writing it requires proof the request came from the user. \
                                      Also (DR-27) `BIOROUTER_PRIVACY_MIXING_POLICY`, which is \
                                      user-only in every mode. Also a key that decides where a \
                                      provider sends its requests and credentials (a host, an \
                                      endpoint), when the write would change what it resolves to"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn upsert_config(
    // Before `Json`, which consumes the body and must be last.
    headers: http::HeaderMap,
    Json(query): Json<UpsertConfigQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // Issue #56 Task 30, hardening measure (2). The master switch is the ONE key
    // this route will not write as an ordinary config value.
    //
    // ⚠ These two arms look contradictory and are not. `/config/upsert` MUST be
    // one of the toggle's two writers — it is the channel Settings > Privacy
    // uses — and a BARE upsert of this key MUST be refused. What separates them
    // is the confirmation field, which is what the panel sends and what a tool
    // call composing an ordinary config write does not.
    if biorouter::privacy::is_privacy_tiers_key(&query.key) {
        // Exact comparison, deliberately: a case-insensitive or trimmed match
        // would let "disable privacy tiers" through, and the phrase exists to be
        // typed rather than guessed.
        if query.confirm.as_deref() != Some(biorouter::privacy::PRIVACY_TIERS_DISABLE_PHRASE) {
            return Err((StatusCode::FORBIDDEN, master_switch_refusal(&query.key)));
        }
        // ⚠ And never into the SECRET store. `config.set(.., is_secret)` routes a
        // secret to the OS credential store, which the start-up loader does not
        // read — so a confirmed secret write would set this process's atomic to
        // `off` and then silently revert to `on` at the next launch, with the
        // panel showing whichever of the two it last read. Unreachable from the
        // panel, which always sends `false`; refused here so that stays a
        // property of the daemon rather than of one caller.
        if query.is_secret {
            return Err((
                StatusCode::FORBIDDEN,
                format!(
                    "'{}' is the master privacy switch and cannot be stored as a secret: the \
                     daemon reads it from its own record in the configuration directory at \
                     start-up and would not see a value written to the credential store.",
                    query.key
                ),
            ));
        }
    }
    // Issue #56 DR-16, open question 24. `/config/upsert` writes ANY key, and a
    // handful of them decide what capability the next session comes up with —
    // `restore_provider_from_session` falls back to the config provider, so a
    // write here is a tier raise with no `/agent/update_provider` call at all.
    // DR-14 already makes config.yaml a filesystem deny root for the same
    // reason; this is the HTTP channel to the same file.
    //
    // Key-scoped, NOT blanket: the GUI writes config on nearly every settings
    // interaction, and a rule that fires constantly is a rule people route
    // around.
    // DR-15's master opt-out, read INSIDE the gate. A direct read, not a
    // `CallCapability`: an HTTP config write is not a tool call and has no
    // admitted capability to inherit.
    if biorouter::privacy::privacy_tiers_enabled()
        && biorouter::privacy::is_capability_key(&query.key)
        && !is_user_action(&headers)
    {
        return Err((
            StatusCode::CONFLICT,
            PrivacyRefusal::CapabilityConfigNeedsUser {
                key: query.key.clone(),
            }
            .to_string(),
        ));
    }
    let write = DestinationChange::Write(&query.value);
    if let Some(refusal) =
        destination_change_refusal(Config::global(), &query.key, write, &headers).await
    {
        return Err(refusal);
    }

    let config = Config::global();

    // Issue #56 Task 42, DR-22. The master switch does NOT go through
    // `config.set` — its home is its own record beside `config.yaml`, and this
    // route is the only thing in the tree that writes it.
    //
    // ⚠ **A copy left in `config.yaml` would defeat the move.** Task 30 closed
    // the HTTP channel to this key, but DR-17 descoped the filesystem barrier
    // that DR-14 had put around `config.yaml`, so writing the key into that file
    // by hand stayed a next-launch disable, and "only on restart" is not a
    // control, because daemons restart routinely and a model can wait. Writing
    // the value here and *also* persisting it there would keep both files
    // agreeing today and hand the retired key its meaning back tomorrow.
    if biorouter::privacy::is_privacy_tiers_key(&query.key) {
        // Parsed through the same function the loader uses, so the running
        // daemon and the next start-up can never disagree about what was asked
        // for.
        let on = biorouter::privacy::privacy_tiers_value_is_on(&query.value).unwrap_or(true);

        // Issue #56 DR-20 / Task 55 Step 2. Turning the whole tier system off is
        // at least as consequential as declassifying one chat, so it takes the
        // same operating-system authentication — raised HERE, immediately before
        // the write, so every other refusal this handler can make is already
        // past and the user is not asked for a password to be told afterwards
        // that the request was malformed.
        //
        // ⚠ **Only the OFF direction.** Re-enabling protection is the safe
        // direction, and gating it would mean an `Unavailable` prompter — every
        // headless host, and every Linux install until the packaging ships the
        // polkit action — strands a machine with the feature disabled and no way
        // to turn it back on. That is the same asymmetry Task 55 Step 1 applies
        // to a `turn:*` chat: spend the cost where the consequence is.
        let mut confirmation = biorouter::privacy::master_switch::Confirmation {
            system_authenticated: false,
            // Recorded, not required — see note (c) in privacy-tiers.md
            // §12.2 for why this arm does not demand the header. The stamp says
            // whether it came, so an audit can tell the app's own window from a
            // caller holding only the daemon secret.
            user_action: is_user_action(&headers),
        };
        if !on {
            let prompter = biorouter::privacy::system_auth::prompter();
            let request = biorouter::privacy::system_auth::AuthRequest::about(
                MASTER_SWITCH_AUTH_REASON,
                MASTER_SWITCH_AUTH_SUBJECT,
            );
            // Bounded (P-05). This `.await` used to be unbounded, and on macOS
            // it never returned: the route sent no response at all and the
            // panel's `busy` state was permanent.
            if let Some(refusal) =
                biorouter::privacy::system_auth::authenticate_or_refuse(prompter, &request).await
            {
                // Nothing has been written at this point — not the record, not
                // the live atomic — so the feature is left exactly as it was, in
                // the enforcing direction.
                return Err((
                    StatusCode::FORBIDDEN,
                    format!("{MASTER_SWITCH_AUTH_REFUSED} {refusal}"),
                ));
            }
            confirmation.system_authenticated = true;
        }

        return match biorouter::privacy::master_switch::write_for(config, on, confirmation) {
            Ok(report) => {
                // Hardening measure (3): the authoritative value lives in daemon
                // memory, so the write to disk is not enough — this is the
                // SECOND of the toggle's two writers (the first is start-up's
                // `load_privacy_tiers_from_config`).
                biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(on);
                // H3: and the report moves with the value, by the same writer,
                // so the surface says "Settings > Privacy" the moment it lands
                // rather than repeating what the last launch loaded.
                biorouter::privacy::master_switch::remember(report);
                Ok(Json(Value::String(format!("Upserted key {}", query.key))))
            }
            // The live value is deliberately NOT moved when the record could not
            // be written: a switch that flips for this process and reverts at the
            // next launch is the divergence Task 30's measure (3) exists to
            // prevent, and the user would be told it worked.
            Err(e) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "Failed to record the master privacy switch: {e}. The setting was not \
                     changed."
                ),
            )),
        };
    }

    // Issue #56 Task 52, DR-27. The cross-institution mixing policy — the second
    // setting that does NOT go through `config.set`, and for DR-22's reason
    // verbatim: DR-17 left `config.yaml` agent-writable, so a security control
    // stored there is one a public model can set to `open` and have obeyed at the
    // next launch. Its home is its own record beside `config.yaml`, and this
    // route is the only thing in the tree that writes it.
    if biorouter::privacy::mixing::is_mixing_policy_key(&query.key) {
        return upsert_mixing_policy(config, &query, &headers).await;
    }

    if let Some(refusal) = config_value_refusal(&query.key, &query.value) {
        return Err((StatusCode::BAD_REQUEST, refusal));
    }
    if names_a_provider(&query.key) {
        let registered = || async {
            get_providers()
                .await
                .into_iter()
                .map(|(metadata, _)| metadata.name)
                .collect::<Vec<String>>()
        };
        if unknown_provider_refusal(&query.key, &query.value, &registered().await).is_some() {
            // A custom provider another process added (`biorouter configure`)
            // is on disk but not yet in this daemon's registry: read the
            // custom providers again before calling the name unknown.
            if let Err(error) = biorouter::providers::refresh_custom_providers().await {
                tracing::warn!("could not re-read custom providers: {error}");
            }
            if let Some(refusal) =
                unknown_provider_refusal(&query.key, &query.value, &registered().await)
            {
                return Err((StatusCode::BAD_REQUEST, refusal));
            }
        }
    }

    let result = config.set(&query.key, &query.value, query.is_secret);

    match result {
        Ok(_) => Ok(Json(Value::String(format!("Upserted key {}", query.key)))),
        Err(_) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to upsert key {}", query.key),
        )),
    }
}

/// What an HTTP config route is about to do to a key.
#[derive(Clone, Copy)]
enum DestinationChange<'a> {
    Write(&'a Value),
    Remove { is_secret: bool },
}

/// Why `/config/upsert` or `/config/remove` refuses to change `key`, or `None`
/// when it may go ahead.
///
/// A key in [`biorouter::providers::destination_keys`] decides where a provider
/// sends its requests, and with them its saved key or this computer's own
/// sign-in. A chat on that provider, `GET /config/providers/{name}/models` and
/// a live `/config/check_provider` all send the credential wherever the key
/// points, so a caller holding only the daemon secret (which a public chat's
/// shell can recover) that could move it would decide who receives the key.
/// Changing one therefore takes the proof of a person, as the capability keys
/// do (DR-16). Unlike them it does not wait on the privacy master switch: it
/// protects a credential in every tier, as the secret guard does.
///
/// A daemon that holds no user-action key (`biorouter serve`, or the desktop's
/// fault state) can prove no one, so there the key is changed on the computer
/// itself. A change that leaves the key resolving to what it resolves to now is
/// not refused: a settings form re-saves an untouched host beside a new key,
/// and that moves nothing.
///
/// ⚠ This closes the HTTP door only. `config.yaml` itself is still writable by
/// the agent's shell (DR-14's filesystem deny is deferred), and every sender
/// above follows the file.
async fn destination_change_refusal(
    config: &Config,
    key: &str,
    change: DestinationChange<'_>,
    headers: &http::HeaderMap,
) -> Option<(StatusCode, String)> {
    if !biorouter::providers::is_destination_key(key) {
        return None;
    }
    let reader = RefusalReader::of(headers)?;
    let defaults = declared_defaults(key).await;
    (!leaves_destination_unchanged(config, key, change, &defaults))
        .then(|| (StatusCode::CONFLICT, destination_refusal(key, reader)))
}

/// Who reads a refusal to move where a provider's key goes, which decides what
/// it should say. The gate is the same for all three; only the sentence
/// differs, because the sentence written for a model in a chat (go and ask the
/// user) sent a person in a browser back to the page they were on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefusalReader {
    /// A daemon that holds a user-action key, and a caller that did not present
    /// it. The desktop app sends the key with every settings write, so this is
    /// a script or a model holding the daemon secret.
    Agent,
    /// A daemon that holds no key and was never handed one: `biorouter serve`,
    /// whose reader is a person in a browser, or a hand-run `biorouterd`.
    HostComputer,
    /// The desktop app's own daemon, started without the key the app meant to
    /// hand it. A person at the desktop, and a fault a restart repairs.
    DesktopWithoutItsKey,
}

impl RefusalReader {
    /// `None` for a caller who proved a person asked, whom nothing refuses.
    fn of(headers: &http::HeaderMap) -> Option<Self> {
        match biorouter_server::auth::user_action_proof(headers) {
            biorouter_server::auth::UserActionProof::Proven => None,
            biorouter_server::auth::UserActionProof::Unproven => Some(Self::Agent),
            biorouter_server::auth::UserActionProof::NoKeyInstalled => {
                Some(if biorouter_server::launch::expected_a_user_action_key() {
                    Self::DesktopWithoutItsKey
                } else {
                    Self::HostComputer
                })
            }
        }
    }
}

/// Why this daemon cannot tell who asked, for the two readers who are people.
const CANNOT_CONFIRM_A_PERSON: &str = "This Biorouter cannot confirm who makes a change, and a \
     browser opened with `biorouter serve` never can";
const DESKTOP_KEY_MISSING: &str = "Biorouter cannot confirm that this change came from you: the \
     app that started it was meant to hand it a key for that, and none arrived";

/// The refusal `/config/upsert` and `/config/remove` give for a destination key.
fn destination_refusal(key: &str, reader: RefusalReader) -> String {
    let what = format!(
        "'{key}' decides where a provider sends its requests and the key or sign-in they carry"
    );
    match reader {
        RefusalReader::Agent => format!(
            "{what}, so changing it is the user's decision, and this request carried no proof it \
             came from them. Nothing was changed. The user can change it in the provider's \
             settings in the Biorouter app, or with `biorouter configure` on the computer running \
             Biorouter."
        ),
        RefusalReader::HostComputer => format!(
            "{what}. {CANNOT_CONFIRM_A_PERSON}, so this setting is changed on the computer running \
             Biorouter: run `biorouter configure` there, or use the Biorouter desktop app on that \
             computer. Nothing was changed."
        ),
        RefusalReader::DesktopWithoutItsKey => format!(
            "{DESKTOP_KEY_MISSING}. {what}, so nothing was changed. Quit and reopen Biorouter, then \
             change it again."
        ),
    }
}

/// Every default a registered provider declares for `key` (`None` for one that
/// declares the key with no default). `AWS_REGION` has two, and they differ.
async fn declared_defaults(key: &str) -> Vec<Option<String>> {
    get_providers()
        .await
        .into_iter()
        .flat_map(|(metadata, _)| metadata.config_keys)
        .filter(|declared| declared.name.eq_ignore_ascii_case(key))
        .map(|declared| declared.default)
        .collect()
}

/// Whether `change` leaves `key` resolving to what it resolves to now. Anything
/// this cannot read counts as a move.
///
/// A write is unchanged when the key already resolves (environment or file) to
/// that value, or resolves to nothing and the value is the default of every
/// provider that declares the key. A removal is unchanged when there is nothing
/// stored where it removes from, or what is stored there is that default.
fn leaves_destination_unchanged(
    config: &Config,
    key: &str,
    change: DestinationChange<'_>,
    defaults: &[Option<String>],
) -> bool {
    let is_every_default = |value: &Value| {
        !defaults.is_empty()
            && defaults
                .iter()
                .all(|default| default.as_deref() == Some(setting_text(value).as_str()))
    };
    let stored = match change {
        DestinationChange::Write(value) => {
            return match config.get_param::<Value>(key) {
                Ok(current) => setting_text(&current) == setting_text(value),
                Err(ConfigError::NotFound(_)) => is_every_default(value),
                Err(_) => false,
            };
        }
        DestinationChange::Remove { is_secret: true } => config.get_secret::<Value>(key),
        DestinationChange::Remove { is_secret: false } => match config.all_values() {
            Ok(values) => values
                .get(key)
                .cloned()
                .ok_or_else(|| ConfigError::NotFound(key.to_string())),
            Err(error) => Err(error),
        },
    };
    match stored {
        Ok(value) => is_every_default(&value),
        Err(ConfigError::NotFound(_)) => true,
        Err(_) => false,
    }
}

/// A setting's value as text, so `11543` and `"11543"` compare equal: the
/// settings form writes a numeric-default key as a number, and an older save
/// left it quoted.
fn setting_text(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_string)
}

/// Why a value cannot be stored under `key`, for the few keys whose shape the
/// daemon knows, or `None` when it may be written.
///
/// `BIOROUTER_MAX_TURNS` is a limit only as a whole number of at least 1.
/// Settings saved `0` whenever its field was cleared (`Number('')` is 0), and a
/// stored 0 stopped every new chat before its first model call; a negative
/// number was stored and silently ignored. The agent now also treats such a
/// stored value as unset, but refusing it here keeps it out of the file at all.
fn config_value_refusal(key: &str, value: &Value) -> Option<String> {
    if key != "BIOROUTER_MAX_TURNS" {
        return None;
    }
    let parsed = match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse::<u64>().ok(),
        _ => None,
    };
    match parsed.filter(|turns| *turns >= 1).map(u32::try_from) {
        Some(Ok(_)) => None,
        _ => Some(format!(
            "Max turns must be a whole number of at least 1, so {value} was not saved."
        )),
    }
}

/// Whether `key` holds a provider NAME: the one new chats start on, or the
/// lead half of a lead/worker pair.
fn names_a_provider(key: &str) -> bool {
    key.eq_ignore_ascii_case("BIOROUTER_PROVIDER")
        || key.eq_ignore_ascii_case("BIOROUTER_LEAD_PROVIDER")
}

/// Why `value` cannot be stored under a key that names a provider (see
/// [`names_a_provider`]), or `None` when it names one of `registered`: the
/// built-in, declarative and custom providers this daemon can build.
///
/// T3-SH-7. Settings' "Edit configuration" writes through `/config/upsert`, and
/// it saved `BIOROUTER_PROVIDER: bogus_provider_qa` with "Configuration
/// updated". Every new chat then failed to start, and the app raised the
/// non-private-model disclosure for a provider that does not exist.
/// `/config/set_provider` already refuses such a name, by building the provider.
fn unknown_provider_refusal(key: &str, value: &Value, registered: &[String]) -> Option<String> {
    let name = value.as_str().map(str::trim).unwrap_or_default();
    if !name.is_empty() && registered.iter().any(|known| known == name) {
        return None;
    }
    let shown = if name.is_empty() {
        value.to_string()
    } else {
        format!("'{name}'")
    };
    Some(format!(
        "{shown} is not a provider Biorouter can use, so {key} was not changed. Choose a \
         provider in Settings > Models."
    ))
}

/// The mixing-policy arm of [`upsert_config`], split out so that handler stays
/// under `clippy::too_many_lines` (issue #56, review round 5). No behaviour
/// change: the guards below run in the order they were written in, and the
/// caller reaches this only for `BIOROUTER_PRIVACY_MIXING_POLICY`.
async fn upsert_mixing_policy(
    config: &'static Config,
    query: &UpsertConfigQuery,
    headers: &http::HeaderMap,
) -> Result<Json<Value>, (StatusCode, String)> {
    // Never into the SECRET store, for the master switch's reason: the
    // credential store is not what the resolver reads, so a secret write
    // would move this process's cached value and silently revert at the next
    // launch.
    if query.is_secret {
        return Err((
            StatusCode::FORBIDDEN,
            format!(
                "'{}' is the cross-institution mixing policy and cannot be stored as a \
                 secret: the daemon reads it from its own record in the configuration \
                 directory and would not see a value written to the credential store.",
                query.key
            ),
        ));
    }
    // DR-19 / DR-27: user-only, in every mode, and NOT conditioned on the
    // master switch being on. Gating this guard on another control's state
    // is the coupling that lets one disabled control disable a second.
    if !is_user_action(headers) {
        return Err((StatusCode::CONFLICT, MIXING_POLICY_NEEDS_USER.to_string()));
    }
    // An unrecognised mode is refused, never resolved to a default: the two
    // wrong answers fail in opposite directions, so there is no safe guess.
    let Some(policy) = biorouter::privacy::mixing::mixing_policy_value(&query.value) else {
        return Err((
            StatusCode::BAD_REQUEST,
            MIXING_POLICY_UNKNOWN_MODE.to_string(),
        ));
    };
    // The direction guard lives inside `set_policy`: loosening raises DR-24's
    // system prompt, tightening raises nothing. Deciding it here would put a
    // second reading of DR-27's ratchet in a route handler.
    match biorouter::privacy::mixing::set_policy(
        config,
        policy,
        &biorouter::privacy::mixing::UserMixingPolicyChange::from_user_action(),
    )
    .await
    {
        Ok(()) => Ok(Json(Value::String(format!("Upserted key {}", query.key)))),
        // Nothing was written on this arm, so the machine is left in the mode
        // it was already in — which is the stricter of the two.
        Err(refused @ biorouter::privacy::mixing::SetPolicyError::Refused(_)) => Err((
            StatusCode::FORBIDDEN,
            format!("{MIXING_POLICY_AUTH_REFUSED} {refused}"),
        )),
        Err(failed) => Err((StatusCode::INTERNAL_SERVER_ERROR, failed.to_string())),
    }
}

/// What the operating system shows above the password field when the user turns
/// the tier system off (issue #56 DR-20 point 4, Task 55 Step 2).
///
/// It states the CONSEQUENCE, not the setting's name. "Change BIOROUTER_PRIVACY_TIERS"
/// is a sentence only the person who wrote the code can act on; a user
/// authorising a system-level change is owed the sentence that tells them what
/// stops happening.
const MASTER_SWITCH_AUTH_REASON: &str =
    "Turn off Biorouter's privacy tiers, so private chats stop being protected.";

/// What the prompt names where a declassification would name its chats.
///
/// ⚠ **Not a session id, and never compared with one.** The master switch has no
/// rows to name and mints no authorisation — the outcome is consumed in the same
/// function that raises the prompt — so there is nothing for a stray id to be
/// matched against. It exists because DR-20 point 4 requires the dialog to say
/// what it authorises, and "the whole install" is a thing to say.
const MASTER_SWITCH_AUTH_SUBJECT: &str = "every private chat on this machine";

/// What `/config/upsert` says when the system authentication for a disable did
/// not happen. The prompter's own sentence is appended, because "you pressed
/// Cancel" and "this machine has no way to raise the prompt" need different
/// advice and only the prompter knows which it was.
const MASTER_SWITCH_AUTH_REFUSED: &str =
    "Turning off Biorouter's privacy tiers needs your operating system to confirm it is you. \
     That did not happen, and the setting was not changed.";

/// What the mixing policy says to a caller that presented no proof of a human
/// (issue #56 Task 52, DR-27 / DR-19).
///
/// It is the model-facing half of the ruling and says the two things a model
/// needs: that this is not its decision, and what to do instead. It forecloses
/// the retry, because a model that reads a refusal as transient loops on it.
const MIXING_POLICY_NEEDS_USER: &str =
    "Biorouter's cross-institution mixing policy is a setting only the person at the keyboard may \
     change, and this request carried no proof it came from them. Nothing was changed. Do not \
     retry; the same call will be refused again, and no setting, hook or permission mode changes \
     it. Tell the user what you need and let them decide.";

/// …and when the value is not one of the three modes.
///
/// Naming all three, because the caller cannot act on "invalid value" and a
/// setting with a closed vocabulary can afford to state it.
const MIXING_POLICY_UNKNOWN_MODE: &str =
    "The cross-institution mixing policy is one of 'open', 'standard' or 'strict'. Nothing was \
     changed.";

/// …and when the system authentication a LOOSENING needs did not happen.
///
/// The prompter's own sentence is appended, because "you pressed Cancel" and
/// "this machine has no way to raise the prompt" need different advice and only
/// the prompter knows which it was — the same shape
/// [`MASTER_SWITCH_AUTH_REFUSED`] has.
const MIXING_POLICY_AUTH_REFUSED: &str =
    "Relaxing Biorouter's cross-institution mixing policy needs your operating system to confirm \
     it is you. That did not happen, and the setting was not changed. Tightening it needs no \
     confirmation at all.";

/// The one sentence both verbs refuse the master switch with. One copy, so the
/// two channels cannot drift into saying different things about the same rule.
fn master_switch_refusal(key: &str) -> String {
    format!(
        "'{key}' is the master privacy switch. It cannot be written or removed as an ordinary \
         configuration value: change it in Settings > App > Privacy, which asks the user to type \
         the confirmation phrase and explains what turning it off exposes."
    )
}

#[utoipa::path(
    post,
    path = "/config/remove",
    request_body = ConfigKeyQuery,
    responses(
        (status = 200, description = "Configuration value removed successfully", body = String),
        (status = 403, description = "Refused: `BIOROUTER_PRIVACY_TIERS` is the master privacy \
                                      switch and may only be changed from Settings > App > \
                                      Privacy, \
                                      never removed, and (issue #56, DR-27) \
                                      `BIOROUTER_PRIVACY_MIXING_POLICY` is set, never deleted"),
        (status = 404, description = "Configuration key not found"),
        (status = 409, description = "Refused by a privacy boundary (issue #56, DR-16): the key \
                                      decides what privacy capability new chats start at, and a \
                                      delete restores its default, so it requires proof the \
                                      request came from the user. Also a key that decides where \
                                      a provider sends its requests and credentials, when \
                                      something other than its default is stored"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn remove_config(
    // Before `Json`, which consumes the body and must be last.
    headers: http::HeaderMap,
    Json(query): Json<ConfigKeyQuery>,
) -> Result<Json<String>, (StatusCode, String)> {
    // Issue #56 Task 30, hardening measure (2) — the same predicate `upsert_config`
    // applies, because "one predicate, both verbs" is the argument DR-16 already
    // made for the capability keys and it holds here for the same reason.
    //
    // Refused OUTRIGHT rather than taking the confirmation phrase: a delete of
    // this key removes it from disk, so the next start-up reads *absent* and
    // resolves to ON while the running daemon keeps whatever its atomic held.
    // Both halves of that divergence are in the safe direction, and there is no
    // legitimate caller — Settings > Privacy writes 'on' or 'off' and never
    // deletes. So the honest answer is "not through this verb", which leaves
    // exactly one way for the value to change and one place to look for it.
    if biorouter::privacy::is_privacy_tiers_key(&query.key) {
        return Err((StatusCode::FORBIDDEN, master_switch_refusal(&query.key)));
    }
    // Issue #56 Task 52, DR-27 — "one predicate, every verb", the argument
    // `is_privacy_tiers_key` already makes. A rule that holds for `/config/upsert`
    // and not for `/config/remove` is a door with one lock, and a delete of this
    // key is not the absence of a write: it is a way to ask for the default back
    // without proving a human or facing the direction guard. It is refused
    // outright rather than gated, because there is no legitimate caller — the
    // panel writes one of the three modes and never deletes.
    if biorouter::privacy::mixing::is_mixing_policy_key(&query.key) {
        return Err((
            StatusCode::FORBIDDEN,
            format!(
                "'{}' is the cross-institution mixing policy and cannot be removed as an \
                 ordinary configuration value: set it to 'open', 'standard' or 'strict' from \
                 Settings > App > Privacy, which is the one door that proves a human and asks \
                 the operating system before it relaxes anything.",
                query.key
            ),
        ));
    }
    // Issue #56 DR-16. The FIFTH channel to the capability keys, and the one the
    // task's own four-channel enumeration missed.
    //
    // A delete is not the absence of a write, it is a write of the DEFAULT.
    // `OLLAMA_HOST` falls back to `localhost` (`providers/ollama.rs`) and
    // `self_hosted_tier` maps loopback to Private, so deleting it moves `ollama`
    // from Public to Private by exactly the mechanism `upsert_config`'s guard
    // exists to block. `LLAMACPP_EXTERNAL_HOST` is the same shape, and deleting
    // `BIOROUTER_LEAD_MODEL` / `BIOROUTER_LEAD_PROVIDER` collapses the
    // lead/worker pair whose tier is the `least()` of two halves — which can
    // only move the result upward.
    //
    // Guarded with the SAME predicate as `upsert_config`, not with the subset
    // that can demonstrably raise: the plan exempted this route on an argument
    // about `BIOROUTER_PROVIDER` alone (delete it and
    // `restore_provider_from_session` finds no provider at all, which is a
    // failure rather than a raise), and an argument that holds for one key in
    // five is not a rule. One predicate, both verbs.
    // DR-15's master opt-out, read INSIDE the gate. A direct read, not a
    // `CallCapability`: an HTTP config write is not a tool call and has no
    // admitted capability to inherit.
    if biorouter::privacy::privacy_tiers_enabled()
        && biorouter::privacy::is_capability_key(&query.key)
        && !is_user_action(&headers)
    {
        return Err((
            StatusCode::CONFLICT,
            PrivacyRefusal::CapabilityConfigNeedsUser {
                key: query.key.clone(),
            }
            .to_string(),
        ));
    }
    // A delete moves a host too: it hands the key back to its default, or to
    // whatever the environment holds. See `destination_change_refusal`.
    let removal = DestinationChange::Remove {
        is_secret: query.is_secret,
    };
    if let Some(refusal) =
        destination_change_refusal(Config::global(), &query.key, removal, &headers).await
    {
        return Err(refusal);
    }

    let config = Config::global();

    let result = if query.is_secret {
        config.delete_secret(&query.key)
    } else {
        config.delete(&query.key)
    };

    match result {
        Ok(_) => Ok(Json(format!("Removed key {}", query.key))),
        Err(_) => Err((
            StatusCode::NOT_FOUND,
            format!("Configuration key {} not found", query.key),
        )),
    }
}

/// The one string `POST /config/read` serves in place of a secret.
///
/// Fixed, and carrying **none** of the secret's own bytes. It used to reveal
/// the first `min(len / 2, 8)` characters, so a 40-character key came back as
/// eight real characters followed by asterisks — a partial credential inside
/// the one response whose entire purpose is not to contain one, and a prefix
/// long enough to identify the key and to narrow a search for the rest.
///
/// The LENGTH is fixed for the same reason the bytes are: how long a stored
/// credential is fingerprints which kind it is. Nothing renders this as
/// anything but placeholder text — `DefaultProviderSetupForm.tsx` puts it
/// straight into a field — so there is no caller that needs it to resemble
/// the value.
const SECRET_MASK: &str = "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}";

/// See [`SECRET_MASK`]. The secret is taken and deliberately not looked at:
/// this is the shape a masking helper has to have to be one.
fn mask_secret(_secret: &Value) -> String {
    SECRET_MASK.to_string()
}

#[utoipa::path(
    post,
    path = "/config/read",
    request_body = ConfigKeyQuery,
    responses(
        (status = 200, description = "Configuration value retrieved successfully", body = Value),
        (status = 500, description = "Unable to get the configuration value"),
    )
)]
pub async fn read_config(
    Json(query): Json<ConfigKeyQuery>,
) -> Result<Json<ConfigValueResponse>, StatusCode> {
    if query.key == "model-limits" {
        let limits = ModelConfig::get_all_model_limits();
        return Ok(Json(ConfigValueResponse::Value(
            serde_json::to_value(limits).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        )));
    }

    // Issue #56 Task 42, DR-22.
    if biorouter::privacy::is_privacy_tiers_key(&query.key) {
        return Ok(Json(ConfigValueResponse::Value(privacy_tiers_wire_value())));
    }
    // H3 — and on both read paths, for the reason the mixing arm below gives.
    if biorouter::privacy::is_privacy_tiers_record_key(&query.key) {
        return Ok(Json(ConfigValueResponse::Value(
            privacy_tiers_record_wire_value(),
        )));
    }

    // Issue #56 Task 52, DR-27 — and this arm is not optional. The value is not
    // in `config.yaml`, so without it `config.get` answers `NotFound` → `null`,
    // and a panel that just wrote 'strict' would render "no mode set". Telling
    // the user something false about the control they have just used is the
    // failure `privacy_tiers_value_is_on`'s doc names, and it costs one branch to
    // avoid.
    if biorouter::privacy::mixing::is_mixing_policy_key(&query.key) {
        return Ok(Json(ConfigValueResponse::Value(mixing_policy_wire_value())));
    }

    let config = Config::global();

    let response_value = match config.get(&query.key, query.is_secret) {
        Ok(value) => {
            if query.is_secret {
                ConfigValueResponse::MaskedValue(MaskedSecret {
                    masked_value: mask_secret(&value),
                })
            } else {
                ConfigValueResponse::Value(value)
            }
        }
        Err(ConfigError::NotFound(_)) => ConfigValueResponse::Value(Value::Null),
        Err(_) => {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    Ok(Json(response_value))
}

#[utoipa::path(
    get,
    path = "/config/extensions",
    responses(
        (status = 200, description = "All extensions retrieved successfully", body = ExtensionResponse),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_extensions() -> Result<Json<ExtensionResponse>, StatusCode> {
    let extensions = biorouter::config::get_all_extensions();
    let warnings = biorouter::config::get_warnings();
    Ok(Json(ExtensionResponse {
        extensions,
        warnings,
    }))
}

#[utoipa::path(
    post,
    path = "/config/extensions",
    request_body = ExtensionQuery,
    responses(
        (status = 200, description = "Extension added or updated successfully", body = String),
        (status = 400, description = "Invalid request"),
        (status = 422, description = "Could not serialize config.yaml"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn add_extension(
    Json(extension_query): Json<ExtensionQuery>,
) -> Result<Json<String>, StatusCode> {
    let extensions = biorouter::config::get_all_extensions();
    let key = biorouter::config::extensions::name_to_key(&extension_query.name);

    let is_update = extensions.iter().any(|e| e.config.key() == key);

    biorouter::config::set_extension(ExtensionEntry {
        enabled: extension_query.enabled,
        config: extension_query.config,
    });

    if is_update {
        Ok(Json(format!("Updated extension {}", extension_query.name)))
    } else {
        Ok(Json(format!("Added extension {}", extension_query.name)))
    }
}

#[utoipa::path(
    delete,
    path = "/config/extensions/{name}",
    responses(
        (status = 200, description = "Extension removed successfully", body = String),
        (status = 404, description = "Extension not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn remove_extension(Path(name): Path<String>) -> Result<Json<String>, StatusCode> {
    let key = biorouter::config::extensions::name_to_key(&name);
    biorouter::config::remove_extension(&key);
    Ok(Json(format!(
        "Removed extension {}. Saved credentials were retained and may be reused on reinstall.",
        name
    )))
}

#[derive(Deserialize, ToSchema)]
pub struct PurgeExtensionCredentialsRequest {
    pub keys: Vec<String>,
}

fn require_credential_user(headers: &http::HeaderMap) -> Result<(), (StatusCode, String)> {
    if headers.contains_key("X-Caller-Provider") || !is_user_action(headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "Managing saved credentials requires a user action in Settings".to_string(),
        ));
    }
    Ok(())
}

async fn extension_provider_credential_references(
) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut references = std::collections::BTreeMap::<String, Vec<String>>::new();
    for (provider, _) in get_providers().await {
        for key in provider.config_keys {
            references
                .entry(key.name.to_uppercase())
                .or_default()
                .push(format!("Provider: {}", provider.name));
        }
    }
    references
}

#[utoipa::path(
    get,
    path = "/config/extensions/{name}/credentials",
    responses(
        (status = 200, description = "Saved credential names and sharing safeguards", body = Vec<ExtensionCredential>),
        (status = 403, description = "User action required"),
        (status = 409, description = "Credential references could not be verified")
    )
)]
pub async fn get_extension_credentials(
    headers: http::HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<Vec<ExtensionCredential>>, (StatusCode, String)> {
    require_credential_user(&headers)?;
    let references = extension_provider_credential_references().await;
    Config::global().extension_credentials(&name, &references, None)
        .map(Json).map_err(|_| (StatusCode::CONFLICT,
            "Cannot verify saved credentials; reload Settings and check extension configuration".to_string()))
}

#[utoipa::path(
    post,
    path = "/config/extensions/{name}/credentials/purge",
    request_body = PurgeExtensionCredentialsRequest,
    responses(
        (status = 200, description = "Selected unshared saved credentials deleted", body = Vec<ExtensionCredential>),
        (status = 403, description = "User action required"),
        (status = 409, description = "Credential references changed or deletion failed")
    )
)]
pub async fn purge_extension_credentials(
    headers: http::HeaderMap,
    Path(name): Path<String>,
    Json(request): Json<PurgeExtensionCredentialsRequest>,
) -> Result<Json<Vec<ExtensionCredential>>, (StatusCode, String)> {
    require_credential_user(&headers)?;
    let references = extension_provider_credential_references().await;
    Config::global().extension_credentials(&name, &references, Some(&request.keys))
        .map(Json).map_err(|_| (StatusCode::CONFLICT,
            "Credentials could not be deleted: references changed, a key is shared or protected, or storage is unavailable. Review saved credentials again.".to_string()))
}

#[utoipa::path(
    get,
    path = "/config",
    responses(
        (status = 200, description = "All configuration values retrieved successfully", body = ConfigResponse)
    )
)]
pub async fn read_all_config() -> Result<Json<ConfigResponse>, StatusCode> {
    let config = Config::global();

    let mut values = config
        .all_values()
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;

    // Issue #56 Task 42, DR-22.
    values.insert(
        biorouter::privacy::PRIVACY_TIERS_CONFIG_KEY.to_string(),
        privacy_tiers_wire_value(),
    );
    // H3. INSERTED, not merged: a copy of this key in `config.yaml` — which
    // `/config/upsert` writes for any key — is replaced here, never passed on.
    values.insert(
        biorouter::privacy::PRIVACY_TIERS_RECORD_KEY.to_string(),
        privacy_tiers_record_wire_value(),
    );
    // Issue #56 Task 52, DR-27 — both read paths, for the reason the single-key
    // one gives: the value is not in `config.yaml`, so a bulk read that skipped
    // it would report the setting as absent on every machine.
    values.insert(
        biorouter::privacy::mixing::MIXING_POLICY_CONFIG_KEY.to_string(),
        mixing_policy_wire_value(),
    );

    Ok(Json(ConfigResponse { config: values }))
}

/// The mixing policy as the two config READ paths report it (issue #56, DR-27).
///
/// ⚠ **The LIVE value, the same one every gate reads — never a second read of
/// the record.** A panel showing the file while the daemon enforces its cached
/// value is Task 30's hardening measure (3) seen from the reading end: the user
/// is told what will apply after the next restart rather than what is applying
/// now, and only one of those is the control they just used.
///
/// A string, because that is what the panel writes back and what
/// [`biorouter::privacy::mixing::MixingPolicy::parse`] round-trips.
fn mixing_policy_wire_value() -> Value {
    Value::String(biorouter::privacy::mixing::policy().as_str().to_string())
}

/// The master switch as the two config READ paths report it (issue #56, DR-22).
///
/// ⚠ **Sourced from the live value, and it overrides whatever `config.yaml`
/// holds.** DR-22 moved the switch's home out of that file; the key can still
/// appear there — a hand edit, a restored backup, an install that predates the
/// migration — and it means nothing. Passing such a value through to the
/// renderer would paint Settings → Privacy and every badge in the app with a
/// state the daemon is not in, which is precisely the failure
/// `privacy_tiers_value_is_on`'s own doc-comment refuses: telling the user
/// something false about the control they just used.
///
/// The live atomic rather than the record on disk, because the atomic is what
/// every gate actually consults (Task 30's hardening measure (3)) — the panel
/// must report what is enforcing, not what will enforce after the next restart.
///
/// A string rather than a bool because that is what the panel writes back and
/// what both value parsers — Rust's and `privacyTiers.ts`'s — round-trip.
fn privacy_tiers_wire_value() -> Value {
    Value::String(
        if biorouter::privacy::privacy_tiers_enabled() {
            "on"
        } else {
            "off"
        }
        .to_string(),
    )
}

/// The switch's record report as the two config READ paths serve it (H3, the
/// 2026-09-10 security test drive): where the record is and which door last
/// wrote it, so the app can say "off, and turned off outside the app" instead
/// of nothing.
///
/// ⚠ **From memory, never a second read of the record** — for
/// [`privacy_tiers_wire_value`]'s reason. The report is what the loader loaded
/// or the confirmed write wrote, remembered beside the atomic by the same two
/// writers; a fresh read of the file would describe what the NEXT launch will
/// do, which is not the control in force.
///
/// ⚠ **`null` when the report does not describe the live value** — a process
/// that never loaded the switch, or a test that moved the atomic directly. The
/// renderer then shows the off-state without an explanation; that loses the
/// "how", and it never loses the notice, whose visibility is the switch's alone.
fn privacy_tiers_record_wire_value() -> Value {
    match biorouter::privacy::master_switch::remembered() {
        Some(report) if report.enabled == biorouter::privacy::privacy_tiers_enabled() => {
            serde_json::to_value(report).unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}

/// How long one provider gets to construct itself before its affiliation is
/// given up on.
///
/// Construction is supposed to be config reads, but it is not guaranteed to be:
/// `bedrock.rs` runs the AWS default credential chain, which can reach for IMDS
/// and sit on a connect timeout. A listing route may not inherit that. Giving up
/// costs a badge (`None`, rendered as nothing) and never a claim.
const AFFILIATION_RESOLVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// DR-26's third axis for one provider, taken off a live instance (issue #56).
///
/// ⚠ **It builds the provider through `providers::create` — the same function
/// `POST /agent/update_provider` calls before reading `new_provider.affiliation()`
/// for the grant lookup.** That is the point: this route must answer with the
/// affiliation of the instance that *would be bound*, not with a claim derived
/// from the provider's name. A name-keyed table (`versa_* => ucsf`) would keep
/// claiming the institution for a module repointed at another host, which
/// `tier()` has already demoted to Public. `create` also applies the lead/worker
/// interception, so with `BIOROUTER_LEAD_MODEL` set each row reports the
/// **composite's** affiliation — the meet of the pair, which is what binding that
/// row would actually give the chat.
///
/// ⚠ **`None` here means "nothing to show", and it is deliberately the answer to
/// three different questions**: the provider is public (the honest answer — a
/// public model has no third axis), its model config is unusable, or it could
/// not be constructed. The renderer draws nothing for all three, which is the
/// only safe collapse available: an unresolvable provider must not be *given* an
/// affiliation, and the one state that would be lost by drawing nothing —
/// `Unstated`, a private model that names no institution — is reachable only
/// when construction SUCCEEDED, so it is never confused with a failure here.
///
/// ⚠ **Only for a configured provider.** An unconfigured one cannot be
/// constructed (its keys are missing) and cannot be bound to a chat, so building
/// it would buy nothing and would run every provider module's constructor —
/// including the ones with process-global side effects — on a plain GET.
///
/// ⚠ **Both axes come from ONE instance, in one call.** The tier returned
/// beside the affiliation is `Provider::tier()` on the very object
/// `ProviderAffiliation::of` reads, so the row can never pair one endpoint's
/// tier with another's institution — the same reason `ModelsBottomBar` does a
/// single fetch for the pair. Resolving them separately would also double the
/// constructor cost of this route.
async fn resolve_provider_axes(
    metadata: &ProviderMetadata,
) -> (Option<ProviderTier>, Option<ProviderAffiliation>) {
    let unresolved = (None, None);
    let Ok(model) = ModelConfig::new(&metadata.default_model) else {
        return unresolved;
    };
    let created = tokio::time::timeout(
        AFFILIATION_RESOLVE_TIMEOUT,
        biorouter::providers::create(&metadata.name, model),
    )
    .await
    .inspect_err(|_| {
        tracing::warn!(
            provider = %metadata.name,
            "timed out resolving provider tier and affiliation; the row will show neither"
        );
    });
    let Ok(Ok(created)) = created else {
        return unresolved;
    };
    // `tier()` FIRST and off the same borrow, so the pair is one sample of one
    // instance. `ProviderAffiliation::of` asks the same object for the same
    // tier internally, which is what makes the two fields agree by construction
    // rather than by a test that remembers to check.
    (
        Some(created.tier()),
        ProviderAffiliation::of(created.as_ref()),
    )
}

#[utoipa::path(
    get,
    path = "/config/providers",
    responses(
        (status = 200, description = "All configuration values retrieved successfully", body = [ProviderDetails])
    )
)]
pub async fn providers() -> Result<Json<Vec<ProviderDetails>>, StatusCode> {
    let providers = get_providers().await;
    // Concurrently, because each row may construct a provider and a serial pass
    // would add every constructor's latency together on a route the settings
    // grid blocks on.
    let providers_response: Vec<ProviderDetails> = futures::future::join_all(
        providers
            .into_iter()
            .map(|(metadata, provider_type)| provider_details(metadata, provider_type)),
    )
    .await;

    Ok(Json(providers_response))
}

/// The models a provider declares, for the case where it has no live fetch.
///
/// Named and separate because it is the answer the route gives most often, and
/// it used to be `Vec::new()`.
///
/// Measured 2026-09-11 over the 23 registered builtins: **9 do not override
/// `fetch_supported_models`**, so `base.rs`'s `Ok(None)` default is what this
/// route receives for every one of them — and **all nine declare a catalog** the
/// settings grid renders on screen: `azure_openai` 12, `aws_bedrock` 7,
/// `versa_azure` 9, `versa_bedrock` 5, `xai` 9, `snowflake` 8, `zai` 8,
/// `xiaomi_mimo` 4, `sagemaker_tgi` 1. So the route reported "no models" for nine
/// providers that have between one and twelve, under a `200 Models fetched
/// successfully`.
///
/// ⚠ Do not measure this by grepping for `with_models`. That was the first
/// instrument tried here and it gave 6 of 9, because `ProviderMetadata::new` also
/// takes a `model_names` list — `snowflake`, `zai` and `sagemaker_tgi` looked
/// catalogless and are not. Read `known_models` off the live metadata instead.
fn declared_model_names(metadata: &biorouter::providers::base::ProviderMetadata) -> Vec<String> {
    metadata
        .known_models
        .iter()
        .map(|model| model.name.clone())
        .collect()
}

/// One row of `GET /config/providers`.
async fn provider_details(
    metadata: ProviderMetadata,
    provider_type: ProviderType,
) -> ProviderDetails {
    let (is_configured, unavailable_reason) = match provider_readiness(&metadata, provider_type) {
        ProviderReadiness::Configured => (true, None),
        ProviderReadiness::NotConfigured => (false, None),
        ProviderReadiness::Unavailable(reason) => (false, Some(reason)),
    };
    // Issue #56, DR-26. Both resolved from the instance, never from the name —
    // see `resolve_provider_axes`.
    let (resolved_tier, affiliation) = if is_configured {
        resolve_provider_axes(&metadata).await
    } else {
        (None, None)
    };

    ProviderDetails {
        name: metadata.name.clone(),
        metadata,
        is_configured,
        provider_type,
        affiliation,
        resolved_tier,
        unavailable_reason,
    }
}

#[utoipa::path(
    get,
    path = "/config/providers/{name}/models",
    params(
        ("name" = String, Path, description = "Provider name (e.g., openai)")
    ),
    responses(
        (status = 200, description = "Models fetched successfully", body = [String]),
        (status = 400, description = "Unknown provider, provider not configured, or authentication error"),
        (status = 429, description = "Rate limit exceeded"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_provider_models(
    Path(name): Path<String>,
) -> Result<Json<Vec<String>>, StatusCode> {
    // ⚠ For a provider with a live listing this sends its saved key (or this
    // computer's own sign-in) to the host its settings name, and it asks for no
    // proof of a person: the model pickers read it on every daemon, `serve`'s
    // browser included, which can prove no one. That is safe only because where
    // the key goes is not the caller's to choose here. The route takes no
    // candidate, and changing a host or endpoint over HTTP takes the proof
    // (`destination_change_refusal`), so the key goes where every chat on that
    // provider already sends it. Adding a parameter that names a host reopens
    // that. ⚠ `config.yaml` itself is still writable by the agent's shell
    // (DR-14's filesystem deny is deferred); a host written there is followed
    // here exactly as it is by every chat, and this route adds no reach to it.
    let loaded_provider =
        biorouter::config::declarative_providers::load_provider(name.as_str()).ok();
    // TODO(Douwe): support a get models url for custom providers
    if let Some(loaded_provider) = loaded_provider {
        return Ok(Json(
            loaded_provider
                .config
                .models
                .into_iter()
                .map(|m| m.name)
                .collect::<Vec<_>>(),
        ));
    }

    let all = get_providers()
        .await
        .into_iter()
        //.map(|(m, p)| m)
        .collect::<Vec<_>>();
    let Some((metadata, provider_type)) = all.into_iter().find(|(m, _)| m.name == name) else {
        return Err(StatusCode::BAD_REQUEST);
    };
    if !check_provider_configured(&metadata, provider_type) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let model_config =
        ModelConfig::new(&metadata.default_model).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let provider = biorouter::providers::create(&name, model_config)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let models_result = retry_operation(&RetryConfig::default(), || async {
        provider.fetch_recommended_models().await
    })
    .await;

    match models_result {
        Ok(Some(models)) => Ok(Json(models)),
        // ⚠ **`None` means "this provider has no LIVE fetch", not "this provider
        // has no models"** — and answering `[]` said the second. Nine of the
        // twenty-three builtins do not override `fetch_supported_models`, so its
        // `Ok(None)` default reached here; ALL NINE declare a catalog the
        // settings grid visibly renders — measured off `known_models`, not by
        // grepping `with_models`, which undercounts by three (see
        // `declared_model_names`). The
        // route was therefore reporting an empty model list for a provider whose
        // models were on screen, under a name and a `200 Models fetched
        // successfully` that both promise the model list.
        //
        // Answering from the declared catalog is not a new policy, it is the one
        // already in force twice over. The declarative-provider branch at the top
        // of this very function returns `config.models` with no live fetch at all;
        // and the desktop's `fetchModelsForProviders` prefers
        // `metadata.known_models` and only falls back to this route when a
        // provider has none. So the fallback WAS the right answer, implemented in
        // the one place that could not help the CLI, an agent, or anything reading
        // the OpenAPI spec.
        //
        // Naming was the alternative, and it was rejected: renaming or
        // redescribing the route regenerates `openapi.json` and the TS client (the
        // `Generated API contract` check), and would still leave every caller
        // holding an empty list for a provider that has models. The shape is
        // unchanged here — same path, same params, same `Vec<String>` body — so no
        // client needs regenerating.
        Ok(None) => Ok(Json(declared_model_names(&metadata))),
        Err(provider_error) => {
            let status_code = match provider_error {
                // Permanent misconfigurations - client should fix configuration
                ProviderError::Authentication(_) => StatusCode::BAD_REQUEST,
                ProviderError::UsageError(_) => StatusCode::BAD_REQUEST,

                // Transient errors - client should retry later
                ProviderError::RateLimitExceeded { .. } => StatusCode::TOO_MANY_REQUESTS,

                // All other errors - internal server error
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };

            tracing::warn!(
                "Provider {} failed to fetch models: {}",
                name,
                provider_error
            );
            Err(status_code)
        }
    }
}

#[utoipa::path(
    get,
    path = "/config/slash_commands",
    responses(
        (status = 200, description = "Slash commands retrieved successfully", body = SlashCommandsResponse)
    )
)]
pub async fn get_slash_commands() -> Result<Json<SlashCommandsResponse>, StatusCode> {
    let mut commands: Vec<_> = slash_commands::list_commands()
        .iter()
        .map(|command| SlashCommand {
            command: command.command.clone(),
            help: command.workflow_path.clone(),
            command_type: CommandType::Workflow,
        })
        .collect();

    for cmd_def in execute_commands::list_commands() {
        commands.push(SlashCommand {
            command: cmd_def.name.to_string(),
            help: cmd_def.description.to_string(),
            command_type: CommandType::Builtin,
        });
    }

    Ok(Json(SlashCommandsResponse { commands }))
}

#[derive(Serialize, ToSchema)]
pub struct PricingData {
    pub provider: String,
    pub model: String,
    pub input_token_cost: f64,
    pub output_token_cost: f64,
    pub cache_read_cost: Option<f64>,
    pub cache_write_cost: Option<f64>,
    pub currency: String,
    pub context_length: Option<u32>,
}

#[derive(Serialize, ToSchema)]
pub struct PricingResponse {
    pub pricing: Vec<PricingData>,
    pub source: String,
}

#[derive(Deserialize, ToSchema)]
pub struct PricingQuery {
    pub provider: String,
    pub model: String,
}

#[utoipa::path(
    post,
    path = "/config/pricing",
    request_body = PricingQuery,
    responses(
        (status = 200, description = "Model pricing data retrieved successfully", body = PricingResponse)
    )
)]
pub async fn get_pricing(
    Json(query): Json<PricingQuery>,
) -> Result<Json<PricingResponse>, StatusCode> {
    let pricing = resolved_provider_model_pricing(&query.provider, &query.model)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(PricingResponse {
        pricing: vec![pricing_data_from_provider_pricing(&query, pricing)],
        source: "resolved".to_string(),
    }))
}

fn pricing_data_from_provider_pricing(
    query: &PricingQuery,
    pricing: ProviderModelPricing,
) -> PricingData {
    PricingData {
        provider: query.provider.clone(),
        model: query.model.clone(),
        input_token_cost: pricing.input_token_cost,
        output_token_cost: pricing.output_token_cost,
        cache_read_cost: pricing.cache_read_cost,
        cache_write_cost: pricing.cache_write_cost,
        currency: pricing.currency,
        context_length: pricing.context_length,
    }
}

#[utoipa::path(
    post,
    path = "/config/init",
    responses(
        (status = 200, description = "Config initialization check completed", body = String),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn init_config() -> Result<Json<String>, StatusCode> {
    let config = Config::global();

    if config.exists() {
        return Ok(Json("Config already exists".to_string()));
    }

    // Use the shared function to load init-config.yaml
    match biorouter::config::base::load_init_config_from_workspace() {
        Ok(init_values) => match config.initialize_if_empty(init_values) {
            Ok(_) => Ok(Json("Config initialized successfully".to_string())),
            Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        },
        Err(_) => Ok(Json(
            "No init-config.yaml found, using default configuration".to_string(),
        )),
    }
}

#[utoipa::path(
    post,
    path = "/config/permissions",
    request_body = UpsertPermissionsQuery,
    responses(
        (status = 200, description = "Permission update completed", body = String),
        (status = 400, description = "Invalid request"),
    )
)]
pub async fn upsert_permissions(
    Json(query): Json<UpsertPermissionsQuery>,
) -> Result<Json<String>, StatusCode> {
    let permission_manager = biorouter::config::PermissionManager::instance();

    for tool_permission in &query.tool_permissions {
        permission_manager.update_user_permission(
            &tool_permission.tool_name,
            tool_permission.permission.clone(),
        );
    }

    Ok(Json("Permissions updated successfully".to_string()))
}

#[utoipa::path(
    post,
    path = "/config/detect-provider",
    request_body = DetectProviderRequest,
    responses(
        (status = 200, description = "Detection result (provider_name is null with a reason on failure)", body = DetectProviderResponse),
    )
)]
pub async fn detect_provider(
    Json(detect_request): Json<DetectProviderRequest>,
) -> Json<DetectProviderResponse> {
    let api_key = detect_request.api_key.trim();

    // The detection engine probes candidate /models endpoints using a task-local
    // config override (never mutating the process env) and returns either the
    // validated provider or a classified failure reason.
    match detect_provider_from_api_key(api_key).await {
        Ok(detected) => Json(DetectProviderResponse {
            provider_name: Some(detected.provider),
            api_key_config_key: Some(detected.api_key_config_key),
            models: detected.models,
            default_model: detected.default_model,
            extra_config: detected.extra_config,
            reason: None,
        }),
        Err(err) => Json(DetectProviderResponse {
            provider_name: None,
            api_key_config_key: None,
            models: Vec::new(),
            default_model: None,
            extra_config: HashMap::new(),
            // "timeout" | "network" | "invalid_key" | "no_match"
            reason: Some(err.code().to_string()),
        }),
    }
}

#[utoipa::path(
    get,
    path = "/config/detectable-providers",
    responses(
        (status = 200, description = "Providers supported by API-key auto-detection", body = DetectableProvidersResponse),
    )
)]
pub async fn get_detectable_providers() -> Json<DetectableProvidersResponse> {
    // Single source of truth: the detectable set lives in `auto_detect`; we only
    // enrich it with display names from provider metadata here.
    let metadata = get_providers().await;
    let providers = detectable_providers()
        .into_iter()
        .map(|name| {
            let display_name = metadata
                .iter()
                .find(|(m, _)| m.name == name)
                .map(|(m, _)| m.display_name.clone())
                .unwrap_or_else(|| name.to_string());
            DetectableProvider {
                name: name.to_string(),
                display_name,
            }
        })
        .collect();

    Json(DetectableProvidersResponse { providers })
}

#[utoipa::path(
    post,
    path = "/config/backup",
    responses(
        (status = 200, description = "Config file backed up", body = String),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn backup_config() -> Result<Json<String>, StatusCode> {
    let config_path = Paths::config_dir().join("config.yaml");

    if config_path.exists() {
        let file_name = config_path
            .file_name()
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

        let mut backup_name = file_name.to_os_string();
        backup_name.push(".bak");

        let backup = config_path.with_file_name(backup_name);
        match std::fs::copy(&config_path, &backup) {
            Ok(_) => Ok(Json(format!("Copied {:?} to {:?}", config_path, backup))),
            Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
    } else {
        Err(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

/// What `POST /config/recover` actually did, in a form a caller can act on.
///
/// ⚠ `message` is the sentence to show a person and `persisted` is the answer to
/// decide on; the two must never be collapsed. A caller that substring-matches
/// the warning back out of `message` is one rewording away from silently
/// concluding the opposite — which is what the route invited, because it used to
/// return that sentence and nothing else.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConfigRecoveryReport {
    /// One sentence, ready to show, covering everything the fields below say.
    pub message: String,
    /// The config keys this process is now running on.
    pub recovered_keys: Vec<String>,
    /// Whether this process's settings are persisting: `config.yaml` holds what
    /// this report describes, and a write to it lands.
    ///
    /// `false` in one of two ways, which `message` spells out:
    /// - the recovery could not write what it recovered: the keys above live
    ///   only in this process, the file on disk is unchanged — still absent, or
    ///   still the contents that would not load — and the next start runs the
    ///   same recovery again;
    /// - `config.yaml` loads, so the keys above are the file's, but it cannot
    ///   be written right now.
    ///
    /// Either way a setting changed in this session will not be saved. Checked
    /// against the disk on every call, so a failure that has since been
    /// repaired is not reported.
    pub persisted: bool,
    /// The write error, verbatim, whenever `persisted` is false.
    pub write_error: Option<String>,
}

/// Build the report from the two things a completed recovery knows.
///
/// Pure, and deliberately separate from the route: the route reads
/// `Config::global()`, a process-wide singleton pointed at the real
/// `~/.config/biorouter`. Every property worth asserting here is of the form
/// *what does the answer say for this outcome?*, and a test that had to arrange
/// the outcome inside the global config could only do it by writing to the
/// user's own config directory.
fn recovery_report(
    recovered_keys: Vec<String>,
    failure: Option<ConfigWriteFailure>,
) -> ConfigRecoveryReport {
    let recovered = if recovered_keys.is_empty() {
        "Config recovery completed, but no data was recoverable. Starting with empty \
         configuration."
            .to_string()
    } else {
        format!(
            "Config recovery completed. Recovered {} keys: {}",
            recovered_keys.len(),
            recovered_keys.join(", ")
        )
    };

    let message = match &failure {
        None => recovered,
        // A recovery that could not WRITE what it recovered leaves the app
        // running on values that vanish at exit. That was silent on the arm a
        // corrupted config with a usable backup actually takes, so this route
        // answered "Recovered 23 keys" for a file it had just failed to write,
        // while the corrupt bytes were still on disk.
        Some(ConfigWriteFailure::ValuesInMemoryOnly(err)) => format!(
            "{recovered} ⚠ These values are in memory only — the config file could not be \
             written ({err}). config.yaml on disk is unchanged, so nothing changed in this \
             session will persist and the next start will recover again."
        ),
        // ⚠ Not the sentence above. The file loads, so the values in use ARE
        // on disk and the next start will not recover anything; only a change
        // made now is at risk. Saying "in memory only" here would be the same
        // kind of false note F2 was.
        Some(ConfigWriteFailure::NotWritable(err)) => format!(
            "{recovered} ⚠ config.yaml loads, but it cannot be written right now ({err}), so \
             a setting changed in this session will not be saved."
        ),
    };

    ConfigRecoveryReport {
        message,
        recovered_keys,
        persisted: failure.is_none(),
        write_error: failure.map(ConfigWriteFailure::into_error),
    }
}

#[utoipa::path(
    post,
    path = "/config/recover",
    responses(
        (status = 200, description = "Config recovery attempted", body = ConfigRecoveryReport),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn recover_config() -> Result<Json<ConfigRecoveryReport>, StatusCode> {
    match run_recovery(Config::global()) {
        Ok(report) => Ok(Json(report)),
        Err(e) => {
            tracing::error!("Config recovery failed: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// The recovery itself, against whichever config it is handed.
///
/// A seam, and the reason it exists is the one `recovery_report` gives for
/// being pure: the route reads `Config::global()`, which is the user's real
/// `~/.config/biorouter`. What this adds over `recovery_report` is the part
/// that depends on the DISK — the reload, and what the config layer says about
/// writing afterwards — and a test can only reach that against a config of its
/// own.
fn run_recovery(config: &Config) -> Result<ConfigRecoveryReport, ConfigError> {
    // This endpoint IS a forced re-read, so it has to force one: the config
    // layer serves a parsed `config.yaml` until the file's stamp moves, and a
    // caller who reaches for "recover" is asking to go back to the disk
    // whatever this process currently believes.
    config.invalidate_values_cache();

    // Force a reload which will trigger recovery if needed
    let values = config.all_values()?;

    // Asked AFTER the reload, never before: the write this reports on may be
    // one the reload itself has just attempted. And asked of the disk, not of
    // a record — a config that loads needs no recovery, so this reload writes
    // nothing, and a failure recorded by an earlier one would otherwise be
    // reported for a file that has since been repaired (finding F2).
    Ok(recovery_report(
        values.keys().cloned().collect(),
        config.outstanding_write_failure(),
    ))
}

#[utoipa::path(
    get,
    path = "/config/validate",
    responses(
        (status = 200, description = "Config validation result", body = String),
        (status = 422, description = "Config file is corrupted")
    )
)]
pub async fn validate_config() -> Result<Json<String>, StatusCode> {
    let config_path = Paths::config_dir().join("config.yaml");

    if !config_path.exists() {
        return Ok(Json("Config file does not exist".to_string()));
    }

    match std::fs::read_to_string(&config_path) {
        Ok(content) => match serde_yaml::from_str::<serde_yaml::Value>(&content) {
            Ok(_) => Ok(Json("Config file is valid".to_string())),
            Err(e) => {
                tracing::warn!("Config validation failed: {}", e);
                Err(StatusCode::UNPROCESSABLE_ENTITY)
            }
        },
        Err(e) => {
            tracing::error!("Failed to read config file: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}
#[utoipa::path(
    post,
    path = "/config/custom-providers",
    request_body = UpdateCustomProviderRequest,
    responses(
        (status = 200, description = "Custom provider created successfully", body = String),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn create_custom_provider(
    Json(request): Json<UpdateCustomProviderRequest>,
) -> Result<Json<String>, (StatusCode, String)> {
    // T3-SH-3: the key typed with it is checked before anything is written. The
    // check sends it, with the typed headers, to the typed URL: nothing saved
    // goes anywhere.
    if !request.api_key.is_empty() {
        let candidate = biorouter::config::declarative_providers::config_for_new_provider(
            &request.engine,
            request.display_name.clone(),
            request.api_url.clone(),
            request.models.clone(),
            request.supports_streaming,
            request.headers.clone(),
        )
        .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
        if let Some(refusal) = typed_key_refusal(candidate, &request.api_key).await {
            return Err((StatusCode::UNPROCESSABLE_ENTITY, refusal));
        }
    }
    let config = biorouter::config::declarative_providers::create_custom_provider(
        &request.engine,
        request.display_name,
        request.api_url,
        request.api_key,
        request.models,
        request.supports_streaming,
        request.headers,
    )
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to create the custom provider".to_string(),
        )
    })?;

    if let Err(e) = biorouter::providers::refresh_custom_providers().await {
        tracing::warn!("Failed to refresh custom providers after creation: {}", e);
    }

    Ok(Json(format!("Custom provider added - ID: {}", config.id())))
}

#[utoipa::path(
    get,
    path = "/config/custom-providers/{id}",
    responses(
        (status = 200, description = "Custom provider retrieved successfully", body = LoadedProvider),
        (status = 404, description = "Provider not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_custom_provider(
    Path(id): Path<String>,
) -> Result<Json<LoadedProvider>, StatusCode> {
    let loaded_provider = biorouter::config::declarative_providers::load_provider(id.as_str())
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Json(loaded_provider))
}

#[utoipa::path(
    delete,
    path = "/config/custom-providers/{id}",
    responses(
        (status = 200, description = "Custom provider removed successfully", body = String),
        (status = 404, description = "Provider not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn remove_custom_provider(Path(id): Path<String>) -> Result<Json<String>, StatusCode> {
    biorouter::config::declarative_providers::remove_custom_provider(&id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if let Err(e) = biorouter::providers::refresh_custom_providers().await {
        tracing::warn!("Failed to refresh custom providers after deletion: {}", e);
    }

    Ok(Json(format!("Removed custom provider: {}", id)))
}

#[utoipa::path(
    put,
    path = "/config/custom-providers/{id}",
    request_body = UpdateCustomProviderRequest,
    responses(
        (status = 200, description = "Custom provider updated successfully", body = String),
        (status = 404, description = "Provider not found"),
        (status = 409, description = "Refused: the update moves the provider to a new URL while \
                                      keeping its saved key or headers, and the request carried \
                                      no proof it came from the user"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn update_custom_provider(
    Path(id): Path<String>,
    // Before `Json`, which consumes the body and must be last.
    headers: http::HeaderMap,
    Json(request): Json<UpdateCustomProviderRequest>,
) -> Result<Json<String>, (StatusCode, String)> {
    if let Some(reader) = RefusalReader::of(&headers) {
        let saved = biorouter::config::declarative_providers::load_provider(&id).ok();
        if let Some(refusal) = custom_provider_move_refusal(saved.as_ref(), &request, reader) {
            return Err(refusal);
        }
    }
    // T3-SH-3: a new key is checked before it replaces the saved one. After the
    // move refusal, so a check never carries a saved key or header to a URL an
    // unproven caller chose. A provider that cannot be loaded is left for the
    // update to report.
    if !request.api_key.is_empty() {
        if let Ok(candidate) = biorouter::config::declarative_providers::config_for_updated_provider(
            &id,
            &request.engine,
            request.api_url.clone(),
        ) {
            if let Some(refusal) = typed_key_refusal(candidate, &request.api_key).await {
                return Err((StatusCode::UNPROCESSABLE_ENTITY, refusal));
            }
        }
    }
    biorouter::config::declarative_providers::update_custom_provider(
        &id,
        &request.engine,
        request.display_name,
        request.api_url,
        request.api_key,
        request.models,
        request.supports_streaming,
    )
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to update custom provider {id}"),
        )
    })?;

    if let Err(e) = biorouter::providers::refresh_custom_providers().await {
        tracing::warn!("Failed to refresh custom providers after update: {}", e);
    }

    Ok(Json(format!("Updated custom provider: {}", id)))
}

/// T3-SH-3 — the sentence refusing a key typed for a declarative or custom
/// provider, when the provider itself rejects it; `None` when it accepted the
/// key, said nothing about it, or did not answer within [`LIVE_CHECK_TIMEOUT`].
async fn typed_key_refusal(
    candidate: biorouter::config::declarative_providers::DeclarativeProviderConfig,
    api_key: &str,
) -> Option<String> {
    let display_name = candidate.display_name.clone();
    let reason = biorouter::config::declarative_providers::typed_key_rejection(
        candidate,
        api_key,
        LIVE_CHECK_TIMEOUT,
    )
    .await?;
    Some(format!(
        "{display_name} rejected this key, so it was not saved: {reason}"
    ))
}

/// Why an update from a caller that could not prove a person is refused, or
/// `None`. The update keeps the provider's saved key when `api_key` is empty,
/// and always keeps its saved headers, which can carry a token too. So moving
/// its URL would send those to the new one: that needs the proof, or the key
/// typed in again with no saved headers to go along. `saved` is `None` when the
/// provider cannot be loaded, which the update itself then reports.
fn custom_provider_move_refusal(
    saved: Option<&LoadedProvider>,
    request: &UpdateCustomProviderRequest,
    reader: RefusalReader,
) -> Option<(StatusCode, String)> {
    let saved = saved?;
    // A provider that is not editable keeps its URL whatever the request says.
    if !saved.is_editable || saved.config.base_url == request.api_url {
        return None;
    }
    let keeps_headers = saved
        .config
        .headers
        .as_ref()
        .is_some_and(|headers| !headers.is_empty());
    // `is_empty`, not a trimmed test: it is the condition the update stores a
    // new key under, so a key of spaces replaces the saved one.
    if !request.api_key.is_empty() && !keeps_headers {
        return None;
    }
    let keeps_key = request.api_key.is_empty();
    let what = if keeps_key {
        "its saved key"
    } else {
        "its saved headers"
    };
    let moving = format!(
        "Moving {} to a new URL would send {what} there",
        saved.config.display_name
    );
    let sentence = match reader {
        RefusalReader::Agent => format!(
            "{moving}. That is the user's decision, and this request carried no proof it came \
             from them. Nothing was changed. The user can change it in the provider's settings \
             in the Biorouter app."
        ),
        // Typing the key again is a way through only when no saved headers
        // would go along with it.
        RefusalReader::HostComputer if keeps_key => format!(
            "{moving}. {CANNOT_CONFIRM_A_PERSON}, so type the key again to move it, or change \
             the URL in the Biorouter desktop app on the computer running Biorouter. Nothing was \
             changed."
        ),
        RefusalReader::HostComputer => format!(
            "{moving}. {CANNOT_CONFIRM_A_PERSON}, so change the URL in the Biorouter desktop app \
             on the computer running Biorouter. Nothing was changed."
        ),
        RefusalReader::DesktopWithoutItsKey => format!(
            "{DESKTOP_KEY_MISSING}. {moving}, so nothing was changed. Quit and reopen Biorouter, \
             then change it again."
        ),
    };
    Some((StatusCode::CONFLICT, sentence))
}

#[utoipa::path(
    post,
    path = "/config/check_provider",
    request_body = CheckProviderRequest,
    responses(
        (status = 200, description = "The provider could be built, and with `live` set its \
                                      credentials were not rejected"),
        (status = 400, description = "The provider could not be built from the saved (or \
                                      candidate) settings, or a candidate named a setting this \
                                      provider does not declare"),
        (status = 403, description = "`live` or `candidate` from a caller that could not prove a \
                                      person asked, on a daemon that holds a user-action key; or, \
                                      on one that holds none, a live check or a candidate that \
                                      names a setting, without typing the key it would be \
                                      checked with"),
        (status = 422, description = "With `live` set: the provider rejected the credentials. \
                                      The body is its message"),
    )
)]
pub async fn check_provider(
    // Before `Json`, which consumes the body and must be last.
    headers: http::HeaderMap,
    Json(CheckProviderRequest {
        provider,
        live,
        candidate,
    }): Json<CheckProviderRequest>,
) -> Result<(), (StatusCode, String)> {
    // A live check sends a credential to the provider's host, and a candidate
    // can name the host. So both need the proof of a person the other
    // credential writes need, on a daemon that holds a user-action key. A daemon
    // holding none (`serve`, or the desktop's fault state) cannot check one, and
    // there the check is not a sender of saved credentials at all: a live check,
    // or a candidate that names a setting, runs only with credentials the caller
    // typed, and every secret it left out is checked as empty
    // (`unproven_check_scope`).
    //
    // ⚠ This fences one door, not the only one. Where a provider sends its
    // saved key is decided by its host and endpoint settings (the AWS endpoint
    // overrides Bedrock and SageMaker read from the stores among them), and a
    // chat, a model listing and this check all send it there. Over HTTP those
    // settings take the same proof (`destination_change_refusal` on
    // `/config/upsert` and `/config/remove`); written straight into
    // `config.yaml` by a shell they do not (DR-14's filesystem deny is
    // deferred), and then every sender follows.
    let proof = biorouter_server::auth::user_action_proof(&headers);
    if let Some(refusal) = credential_check_refusal(live, candidate.is_some(), &proof) {
        return Err(refusal);
    }
    let metadata = get_providers()
        .await
        .into_iter()
        .map(|(metadata, _)| metadata)
        .find(|metadata| metadata.name == provider);
    let overrides = check_overrides(
        metadata.as_ref(),
        &provider,
        candidate,
        live,
        &proof,
        secret_resolves_outside_the_candidate,
    )?;
    let has_secret = metadata
        .as_ref()
        .is_some_and(|metadata| metadata.config_keys.iter().any(|key| key.secret));
    let display_name = metadata.as_ref().map_or_else(
        || provider.clone(),
        |metadata| metadata.display_name.clone(),
    );

    biorouter::config::with_config_overrides(overrides, async {
        let built = create_with_default_model(&provider)
            .await
            .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
        if live && has_secret {
            // 422, not 401: a 401 from this daemon means its own secret was
            // wrong (`check_token`), and this is the provider's answer.
            if let Some(refusal) = live_credential_refusal(&display_name, built.as_ref()).await {
                return Err((StatusCode::UNPROCESSABLE_ENTITY, refusal));
            }
        }
        Ok(())
    })
    .await
}

/// Whether a credential check may run for this caller: see [`check_provider`].
/// Only a check that sends a credential somewhere (`live`) or names new values
/// (`candidate`) asks, and only a daemon that holds a user-action key can refuse.
fn credential_check_refusal(
    live: bool,
    has_candidate: bool,
    proof: &biorouter_server::auth::UserActionProof,
) -> Option<(StatusCode, String)> {
    ((live || has_candidate) && matches!(proof, biorouter_server::auth::UserActionProof::Unproven))
        .then(|| {
            (
                StatusCode::FORBIDDEN,
                "Checking credentials against a provider is the user's decision, and this request \
             did not come from the app's settings."
                    .to_string(),
            )
        })
}

/// The task-local overrides a check runs under, or the refusal: the candidate
/// as [`candidate_overrides`] reads it and, from a caller that could not prove a
/// person, narrowed by [`unproven_check_scope`], with every secret it left out
/// set to an empty value so none of them is read from the store.
fn check_overrides(
    metadata: Option<&ProviderMetadata>,
    provider: &str,
    candidate: Option<HashMap<String, String>>,
    live: bool,
    proof: &biorouter_server::auth::UserActionProof,
    secret_resolves: impl Fn(&str) -> bool,
) -> Result<HashMap<String, String>, (StatusCode, String)> {
    let mut overrides = match candidate {
        Some(values) => candidate_overrides(metadata, provider, values)?,
        None => HashMap::new(),
    };
    if !matches!(proof, biorouter_server::auth::UserActionProof::Proven) {
        for key in unproven_check_scope(metadata, &overrides, live, secret_resolves)? {
            overrides.insert(key, EMPTY_SECRET_OVERRIDE.to_string());
        }
    }
    Ok(overrides)
}

/// An empty secret, in the JSON string form every secret override takes.
const EMPTY_SECRET_OVERRIDE: &str = "\"\"";

/// On a daemon that cannot prove a person (no user-action key), a check that
/// would sign in with something is run only with credentials the caller typed.
/// That is every live check of a provider that declares a secret, since the
/// live call is authenticated, and every check whose candidate names a
/// non-secret setting (a host, an endpoint, a region), since building the
/// provider there can already reach for a sign-in.
///
/// It keeps this route from sending anything the caller did not type: a saved
/// key, or this computer's own sign-in (Azure's Entra login, Databricks'
/// browser login, Google or AWS credentials). It does not decide where a saved
/// key goes on every other path; see [`check_provider`].
///
/// So it is refused when a required secret the caller left out has a value in
/// the environment or the store (`secret_resolves`), since that value is what
/// the provider would sign in with, and when the caller typed no secret at all.
/// Otherwise it returns the declared secrets the caller left out, which the
/// check runs with as empty values. An optional one the caller did not type and
/// that nothing holds is not asked for: it has nothing to send.
fn unproven_check_scope(
    metadata: Option<&ProviderMetadata>,
    overrides: &HashMap<String, String>,
    live: bool,
    secret_resolves: impl Fn(&str) -> bool,
) -> Result<Vec<String>, (StatusCode, String)> {
    let Some(metadata) = metadata else {
        return Ok(Vec::new());
    };
    let secrets: Vec<&ConfigKey> = metadata
        .config_keys
        .iter()
        .filter(|key| key.secret)
        .collect();
    let names_a_setting = metadata
        .config_keys
        .iter()
        .any(|key| !key.secret && overrides.contains_key(&key.name.to_uppercase()));
    // `check_provider` makes its authenticated call only for a provider that
    // declares a secret; for any other, `live` builds and sends nothing.
    let signs_in = live && !secrets.is_empty();
    if !names_a_setting && !signs_in {
        return Ok(Vec::new());
    }
    let left_out: Vec<&ConfigKey> = secrets
        .iter()
        .copied()
        .filter(|key| !overrides.contains_key(&key.name.to_uppercase()))
        .collect();
    let refused = |sentence: String| Err((StatusCode::FORBIDDEN, sentence));
    let display_name = &metadata.display_name;

    let saved: Vec<&str> = left_out
        .iter()
        .filter(|key| key.required && secret_resolves(&key.name))
        .map(|key| key.name.as_str())
        .collect();
    if !saved.is_empty() {
        let (keys, them) = (
            name_list(&saved, "and"),
            if saved.len() == 1 { "it" } else { "them" },
        );
        return refused(if names_a_setting {
            format!(
                "Checking a new {display_name} setting would send the saved {keys} to it. \
                 That is the user's decision, and this daemon cannot confirm the request came \
                 from one, so type {them} in with the setting."
            )
        } else {
            format!(
                "Checking {display_name} live would send the saved {keys} to it. That is the \
                 user's decision, and this daemon cannot confirm the request came from one, so \
                 type {them} in to check {them}."
            )
        });
    }

    let typed_a_secret = secrets.iter().any(|key| {
        overrides
            .get(&key.name.to_uppercase())
            .is_some_and(|value| !secret_override_text(value).trim().is_empty())
    });
    if !typed_a_secret {
        if secrets.is_empty() {
            return refused(format!(
                "Checking a new {display_name} setting is the user's decision, and this daemon \
                 cannot confirm the request came from one."
            ));
        }
        // Name the key the provider signs in with: its required secrets, or,
        // when none is required (Azure, Databricks), whichever it has.
        let required: Vec<&str> = secrets
            .iter()
            .filter(|key| key.required)
            .map(|key| key.name.as_str())
            .collect();
        let wanted = if required.is_empty() {
            let any: Vec<&str> = secrets.iter().map(|key| key.name.as_str()).collect();
            name_list(&any, "or")
        } else {
            name_list(&required, "and")
        };
        let what = if names_a_setting {
            format!("a new {display_name} setting from here needs {wanted} typed in with it")
        } else {
            format!("{display_name} live from here needs {wanted} typed in")
        };
        return refused(format!(
            "Checking {what}. Without that the check would sign in with a saved key or with \
             this computer's own sign-in, which is the user's decision, and this daemon cannot \
             confirm the request came from one."
        ));
    }

    Ok(left_out.iter().map(|key| key.name.to_uppercase()).collect())
}

/// Whether the check would read a value for secret `key` if the candidate left
/// it out: one in the environment or the secret store. A store that cannot be
/// read counts as holding one.
fn secret_resolves_outside_the_candidate(key: &str) -> bool {
    !matches!(
        Config::global().get_secret::<Value>(key),
        Err(ConfigError::NotFound(_))
    )
}

/// A secret override's text: the string inside the JSON literal
/// [`candidate_overrides`] writes, or the value itself.
fn secret_override_text(value: &str) -> String {
    serde_json::from_str::<String>(value).unwrap_or_else(|_| value.to_string())
}

/// `A`, `A and B`, `A, B and C` (or with `or`).
fn name_list(names: &[&str], conjunction: &str) -> String {
    match names {
        [] => String::new(),
        [only] => (*only).to_string(),
        [rest @ .., last] => format!("{} {conjunction} {last}", rest.join(", ")),
    }
}

/// A candidate's values as the task-local overrides the check runs under, or
/// the refusal. Only settings `provider` declares are accepted: a check is
/// about this provider, and an override of anything else (the master privacy
/// switch, another provider's key) is not a candidate for it.
fn candidate_overrides(
    metadata: Option<&ProviderMetadata>,
    provider: &str,
    values: HashMap<String, String>,
) -> Result<HashMap<String, String>, (StatusCode, String)> {
    let Some(metadata) = metadata else {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("There is no provider named '{provider}'."),
        ));
    };
    let mut overrides = HashMap::new();
    for (key, value) in values {
        let Some(declared) = metadata
            .config_keys
            .iter()
            .find(|declared| declared.name.eq_ignore_ascii_case(&key))
        else {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("'{key}' is not a setting of {}.", metadata.display_name),
            ));
        };
        // An override is read the way an environment variable is (JSON, then
        // true/false, then a number), but a secret is saved as a string. So a
        // secret goes in as a JSON string literal, or an all-digit key would be
        // read as a number and fail to build a provider it would have run.
        // `Config::get_secret` and `Config::get_secrets` both take the string
        // back out of the literal, so a provider sees exactly what was typed
        // whichever of the two it reads its key through.
        let value = if declared.secret {
            serde_json::to_string(&value).unwrap_or(value)
        } else {
            value
        };
        overrides.insert(key.to_uppercase(), value);
    }
    Ok(overrides)
}

/// The provider's refusal of its credentials, from one authenticated call, or
/// `None` when it accepted them, has no way to be asked, or could not answer in
/// time. Only an authentication failure refuses: a network error or a missing
/// models endpoint says nothing about the key.
///
/// The call is the provider's own `Provider::check_credentials`: its model
/// listing by default, and for the Versa gateways, which have none, a probe the
/// gateway authenticates without running a model (T3-SH-3). Checking only the
/// listing sent nothing for them, so a wrong Versa key was saved over the
/// working one and shown Configured.
async fn live_credential_refusal(
    display_name: &str,
    provider: &dyn biorouter::providers::base::Provider,
) -> Option<String> {
    live_credential_refusal_within(display_name, provider, LIVE_CHECK_TIMEOUT).await
}

async fn live_credential_refusal_within(
    display_name: &str,
    provider: &dyn biorouter::providers::base::Provider,
    timeout: std::time::Duration,
) -> Option<String> {
    match tokio::time::timeout(timeout, provider.check_credentials()).await {
        Ok(Err(ProviderError::Authentication(message))) => Some(format!(
            "{display_name} rejected these credentials: {}",
            message.trim()
        )),
        _ => None,
    }
}

#[utoipa::path(
    post,
    path = "/config/set_provider",
    request_body = SetProviderRequest,
    responses(
        (status = 200, description = "Default provider and model set"),
        (status = 400, description = "The provider could not be constructed"),
        (status = 409, description = "Refused by a privacy boundary (issue #56, DR-16): this \
                                      route writes BIOROUTER_PROVIDER, which decides what \
                                      privacy capability new chats start at, so it requires \
                                      proof the request came from the user"),
    )
)]
pub async fn set_config_provider(
    // Before `Json`, which consumes the body and must be last.
    headers: http::HeaderMap,
    Json(SetProviderRequest { provider, model }): Json<SetProviderRequest>,
) -> Result<(), (StatusCode, String)> {
    // Issue #56 DR-16. Unconditional on the KEY, unlike `upsert_config`'s
    // key-scoped guard — this route writes BIOROUTER_PROVIDER by construction,
    // so there is no tier-irrelevant call to exempt — but still subject to
    // DR-15's master opt-out, read here inside the gate.
    if biorouter::privacy::privacy_tiers_enabled() && !is_user_action(&headers) {
        return Err((
            StatusCode::CONFLICT,
            PrivacyRefusal::CapabilityConfigNeedsUser {
                key: "BIOROUTER_PROVIDER".to_string(),
            }
            .to_string(),
        ));
    }

    create_with_default_model(&provider)
        .await
        .and_then(|_| {
            // ⚠ ONE write, not two. `set_biorouter_provider` followed by
            // `set_biorouter_model` left `config.yaml` holding the new provider
            // beside the old model — measured at ~55 ms of `versa_azure` next
            // to `gpt-6-astra` — and a chat started in that window binds a pair
            // that was never chosen. The provider decides the session's privacy
            // capability, so a mismatched pair is a privacy-relevant outcome,
            // not only a cosmetic one.
            Config::global()
                .set_biorouter_provider_and_model(provider, model)
                .map_err(|e| anyhow::anyhow!(e))
        })
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    Ok(())
}

/// What `GET /privacy/disclosure` serves (issue #56, DR-17 requirement 3).
///
/// ⚠ **The copy is on the wire on purpose.** The sentence exists in the GUI
/// dialog, the settings panel, the provider grid, the model chip, the CLI,
/// `docs/` and the landing site; four hand-written copies drift within one
/// release and the drifted one is always the one a user reads. One definition
/// lives in `biorouter::privacy::disclosure` and the renderer renders what it is
/// handed — a hardcoded English string in a component is the failure this shape
/// exists to prevent, and it is invisible until the two disagree.
#[derive(Debug, Serialize, ToSchema)]
pub struct PrivacyDisclosureResponse {
    /// The dialog heading, with `{provider}` still in it — the renderer
    /// substitutes the display name of the provider it is warning about, and so
    /// never has to know the English around it.
    pub title_template: String,
    /// The long form: the blocking dialog and the settings panel.
    pub long: String,
    /// The one-line form: the model chip's tooltip and the provider grid's
    /// Commercial section.
    pub short: String,
    /// Settings > App > Privacy's heading and long form: about non-private
    /// models as a class, never "this model" or "this chat" (W2-PRV-14). The
    /// panel shows it whatever model is bound, beside that model's tier.
    pub settings_title: String,
    pub settings: String,
    /// Has the user acknowledged on this install? Once per install, not once per
    /// session — a dialog on every chat is a dialog nobody reads.
    pub acknowledged: bool,
}

/// The disclosure copy, and whether it has been acknowledged.
///
/// ⚠ **Deliberately does NOT consult the master privacy switch.** DR-15 turns
/// off gates, the ratchet and refusals; it does not turn off the truth, and with
/// enforcement off the exposure is *larger*. Every other privacy route in this
/// file reads the switch, which is exactly why wiring this one the same way is
/// the plausible mistake.
#[utoipa::path(
    get,
    path = "/privacy/disclosure",
    responses(
        (status = 200, description = "The one copy of the non-private-model disclosure, plus \
                                      whether this install has acknowledged it",
         body = PrivacyDisclosureResponse),
    )
)]
pub async fn get_privacy_disclosure() -> Json<PrivacyDisclosureResponse> {
    use biorouter::privacy::disclosure;
    Json(PrivacyDisclosureResponse {
        title_template: disclosure::COPY_TITLE_TEMPLATE.to_string(),
        long: disclosure::COPY_LONG.to_string(),
        short: disclosure::COPY_SHORT.to_string(),
        settings_title: disclosure::COPY_SETTINGS_TITLE.to_string(),
        settings: disclosure::COPY_SETTINGS.to_string(),
        acknowledged: disclosure::is_acknowledged(),
    })
}

/// Record that the user has read the disclosure.
///
/// ⚠ **DR-16's proof-of-user, unconditionally.** This is the one thing making
/// DR-17's accepted risks acceptable, so a caller holding nothing but the daemon
/// secret — which AR-11 measured to be recoverable from inside the daemon, i.e.
/// the model — must not be able to acknowledge on the user's behalf. Unlike
/// `upsert_config`'s guard this is NOT additionally gated on the master privacy
/// switch: turning enforcement off must not hand the model the dismiss button.
#[utoipa::path(
    post,
    path = "/privacy/disclosure/ack",
    responses(
        (status = 200, description = "Acknowledged"),
        (status = 403, description = "Refused: acknowledging the disclosure is a user act, and \
                                      this request carried no proof it came from the user"),
        (status = 500, description = "The acknowledgement could not be written"),
    )
)]
pub async fn ack_privacy_disclosure(headers: http::HeaderMap) -> Result<(), (StatusCode, String)> {
    if !is_user_action(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "Acknowledging the non-private-model disclosure is a user action. This request \
             carried no proof that it came from the person at the keyboard."
                .to_string(),
        ));
    }
    biorouter::privacy::disclosure::record_acknowledgement()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/config", get(read_all_config))
        // Issue #56 DR-17 req. 3. Beside the master-switch routes because the
        // panel that shows the switch also shows this, and NOT behind the switch
        // for the reason each handler's doc comment gives.
        .route("/privacy/disclosure", get(get_privacy_disclosure))
        .route("/privacy/disclosure/ack", post(ack_privacy_disclosure))
        .route("/config/upsert", post(upsert_config))
        .route("/config/remove", post(remove_config))
        .route("/config/read", post(read_config))
        .route("/config/extensions", get(get_extensions))
        .route("/config/extensions", post(add_extension))
        .route("/config/extensions/{name}", delete(remove_extension))
        .route(
            "/config/extensions/{name}/credentials",
            get(get_extension_credentials),
        )
        .route(
            "/config/extensions/{name}/credentials/purge",
            post(purge_extension_credentials),
        )
        .route("/config/providers", get(providers))
        .route("/config/providers/{name}/models", get(get_provider_models))
        .route("/config/detect-provider", post(detect_provider))
        .route(
            "/config/detectable-providers",
            get(get_detectable_providers),
        )
        .route("/config/slash_commands", get(get_slash_commands))
        .route("/config/pricing", post(get_pricing))
        .route("/config/init", post(init_config))
        .route("/config/backup", post(backup_config))
        .route("/config/recover", post(recover_config))
        .route("/config/validate", get(validate_config))
        .route("/config/permissions", post(upsert_permissions))
        .route("/config/custom-providers", post(create_custom_provider))
        .route(
            "/config/custom-providers/{id}",
            delete(remove_custom_provider),
        )
        .route("/config/custom-providers/{id}", put(update_custom_provider))
        .route("/config/custom-providers/{id}", get(get_custom_provider))
        .route("/config/check_provider", post(check_provider))
        .route("/config/set_provider", post(set_config_provider))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    /// **A masked secret carries none of the secret.**
    ///
    /// `POST /config/read` with `is_secret: true` answered
    /// `{"maskedValue":"Y2EzNTgy********…"}` — `min(len / 2, 8)` real
    /// characters of the credential, in the one response whose whole purpose is
    /// not to contain one. Eight characters is enough to identify which key is
    /// stored and to narrow a search for the rest.
    ///
    /// The prefix loop is the fail-before: a `!= secret` assertion passes
    /// against the old helper, and so does "contains asterisks".
    #[test]
    fn a_masked_secret_reveals_nothing_of_it() {
        for secret in [
            "ca3582deadbeefcafe0123456789abcdef01234567",
            "sk-proj-AAAABBBBCCCCDDDDEEEEFFFF",
            "short",
            "x",
        ] {
            let masked = super::mask_secret(&serde_json::json!(secret));
            // `chars().take(n)`, not `&secret[..n]`: a byte slice of a string is
            // `clippy::string_slice`, and the property under test is about
            // characters anyway.
            for n in 1..=secret.chars().count() {
                let prefix: String = secret.chars().take(n).collect();
                assert!(
                    !masked.contains(&prefix),
                    "the mask carries the first {n} characters of the secret: {masked}"
                );
            }
            assert!(
                !masked.chars().any(|c| secret.contains(c)),
                "the mask shares characters with the secret: {masked}"
            );
        }

        // …and it is the same length whatever it hides: how long a stored
        // credential is fingerprints which kind it is.
        assert_eq!(
            super::mask_secret(&serde_json::json!("x")),
            super::mask_secret(&serde_json::json!(
                "ca3582deadbeefcafe0123456789abcdef01234567"
            )),
            "the mask's length still leaks the secret's"
        );
        // A non-string secret is masked too, not serialized into the response.
        assert_eq!(
            super::mask_secret(&serde_json::json!({ "token": "abc123" })),
            super::SECRET_MASK
        );
    }

    use http::HeaderMap;

    use super::*;

    /// W2-PRV-2. A check that sends a credential (`live`) or names new values
    /// (`candidate`) needs the proof of a person on a daemon that holds a key; a
    /// plain construction check stays open, as it was.
    #[test]
    fn a_credential_check_needs_a_person_where_one_can_be_proven() {
        use biorouter_server::auth::UserActionProof::{NoKeyInstalled, Proven, Unproven};
        for (live, candidate) in [(true, false), (false, true), (true, true)] {
            let refusal = credential_check_refusal(live, candidate, &Unproven)
                .expect("an unproven caller is refused");
            assert_eq!(refusal.0, StatusCode::FORBIDDEN);
            assert!(credential_check_refusal(live, candidate, &Proven).is_none());
            assert!(credential_check_refusal(live, candidate, &NoKeyInstalled).is_none());
        }
        assert!(credential_check_refusal(false, false, &Unproven).is_none());
    }

    /// W2-PRV-2. A candidate may only name settings the provider declares, and
    /// is looked up the way `get_secret`/`get_param` look overrides up.
    #[test]
    fn a_candidate_names_only_the_providers_own_settings() {
        use biorouter::providers::base::ConfigKey;
        let mut metadata = ProviderMetadata::empty();
        metadata.name = "anthropic".to_string();
        metadata.display_name = "Anthropic".to_string();
        metadata.config_keys = vec![
            ConfigKey::new("ANTHROPIC_API_KEY", true, true, None),
            ConfigKey::new(
                "ANTHROPIC_HOST",
                true,
                false,
                Some("https://api.anthropic.com"),
            ),
        ];

        let overrides = candidate_overrides(
            Some(&metadata),
            "anthropic",
            HashMap::from([("anthropic_api_key".to_string(), "sk-ant-x".to_string())]),
        )
        .expect("a declared key is a candidate");
        assert_eq!(
            overrides.get("ANTHROPIC_API_KEY").map(String::as_str),
            Some("\"sk-ant-x\"")
        );

        for foreign in [
            "BIOROUTER_PRIVACY_TIERS",
            "OPENAI_API_KEY",
            "BIOROUTER_PROVIDER",
        ] {
            let refusal = candidate_overrides(
                Some(&metadata),
                "anthropic",
                HashMap::from([(foreign.to_string(), "x".to_string())]),
            )
            .expect_err("a setting the provider does not declare is refused");
            assert_eq!(refusal.0, StatusCode::BAD_REQUEST, "{foreign}");
        }
        assert!(candidate_overrides(None, "nope", HashMap::new()).is_err());
    }

    /// Review of W2-PRV-2, round 2. Where no person can be proven, a candidate
    /// that names a setting is checked only with what the caller typed. Built on
    /// the providers' REAL metadata, because the refusal a browser on
    /// `biorouter serve` met came from OpenAI's optional second secret.
    #[test]
    fn an_unproven_candidate_is_checked_only_with_what_it_typed() {
        use biorouter::providers::base::Provider;
        use biorouter_server::auth::UserActionProof::{NoKeyInstalled, Proven};
        let openai = biorouter::providers::openai::OpenAiProvider::metadata();
        let check = |metadata: &ProviderMetadata,
                     values: &[(&str, &str)],
                     proof,
                     saved: &[&str]|
         -> Result<HashMap<String, String>, (StatusCode, String)> {
            let saved: Vec<String> = saved.iter().map(|key| key.to_string()).collect();
            // Live, as the settings form always asks.
            check_overrides(
                Some(metadata),
                &metadata.name,
                Some(
                    values
                        .iter()
                        .map(|(key, value)| (key.to_string(), value.to_string()))
                        .collect(),
                ),
                true,
                &proof,
                |key| saved.iter().any(|saved| saved == key),
            )
        };
        let openai_save = [
            ("OPENAI_API_KEY", "sk-typed"),
            ("OPENAI_HOST", "https://api.openai.com"),
            ("OPENAI_BASE_PATH", "v1/chat/completions"),
        ];

        // The failing save: a typed key with the form's required settings, and
        // no custom headers typed or saved. Not refused, and the headers the
        // caller left out are checked as empty.
        let overrides = check(&openai, &openai_save, NoKeyInstalled, &[])
            .expect("a save that carries its key is checked");
        assert_eq!(overrides["OPENAI_API_KEY"], "\"sk-typed\"");
        assert_eq!(overrides["OPENAI_CUSTOM_HEADERS"], EMPTY_SECRET_OVERRIDE);

        // Saved headers (they can carry a token) are not asked for either, and
        // are not read under the host the caller named.
        let overrides = check(
            &openai,
            &openai_save,
            NoKeyInstalled,
            &["OPENAI_CUSTOM_HEADERS"],
        )
        .expect("an optional secret is never demanded");
        assert_eq!(overrides["OPENAI_CUSTOM_HEADERS"], EMPTY_SECRET_OVERRIDE);

        // A host with the saved key left out would send that key there. The
        // sentence names the key it would send, and only that one.
        let host_only = [("OPENAI_HOST", "https://attacker.example")];
        let (status, sentence) = check(&openai, &host_only, NoKeyInstalled, &["OPENAI_API_KEY"])
            .expect_err("the saved key does not go to a named host");
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(sentence.contains("the saved OPENAI_API_KEY"), "{sentence}");
        assert!(!sentence.contains("OPENAI_CUSTOM_HEADERS"), "{sentence}");
        let typed_headers_only = [
            ("OPENAI_HOST", "https://attacker.example"),
            ("OPENAI_CUSTOM_HEADERS", "X-Team=blue"),
        ];
        assert!(
            check(
                &openai,
                &typed_headers_only,
                NoKeyInstalled,
                &["OPENAI_API_KEY"]
            )
            .is_err(),
            "typing an optional secret does not let the saved key travel"
        );
        // With nothing saved it has nothing to send, and is still refused: a
        // check with no typed credential would sign in with none of the caller's.
        let (status, sentence) = check(&openai, &host_only, NoKeyInstalled, &[])
            .expect_err("a named setting needs a typed key");
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(
            sentence.contains("needs OPENAI_API_KEY typed in"),
            "{sentence}"
        );

        // A key alone moves nothing, but a live check still sends only what was
        // typed: the saved headers are checked empty.
        let overrides = check(
            &openai,
            &[("OPENAI_API_KEY", "sk-typed")],
            NoKeyInstalled,
            &["OPENAI_CUSTOM_HEADERS"],
        )
        .unwrap();
        assert_eq!(overrides["OPENAI_CUSTOM_HEADERS"], EMPTY_SECRET_OVERRIDE);
        // Built without the live call, it signs in with nothing, so nothing is
        // blanked.
        let overrides = check_overrides(
            Some(&openai),
            "openai",
            Some(HashMap::from([(
                "OPENAI_API_KEY".to_string(),
                "sk-typed".to_string(),
            )])),
            false,
            &NoKeyInstalled,
            |key| key == "OPENAI_CUSTOM_HEADERS",
        )
        .unwrap();
        assert!(!overrides.contains_key("OPENAI_CUSTOM_HEADERS"));

        // A person who proved it asked is checked with the saved values too.
        let overrides = check(&openai, &host_only, Proven, &["OPENAI_API_KEY"]).unwrap();
        assert_eq!(overrides.len(), 1);

        // LiteLLM has the same optional second secret.
        let litellm = biorouter::providers::litellm::LiteLLMProvider::metadata();
        let overrides = check(
            &litellm,
            &[
                ("LITELLM_API_KEY", "sk-typed"),
                ("LITELLM_HOST", "https://litellm.example"),
            ],
            NoKeyInstalled,
            &["LITELLM_CUSTOM_HEADERS"],
        )
        .expect("a LiteLLM save that carries its key is checked");
        assert_eq!(overrides["LITELLM_CUSTOM_HEADERS"], EMPTY_SECRET_OVERRIDE);

        // Azure signs in with this computer's Entra login when no key is set:
        // an endpoint with the key left out, or typed empty, is refused, since
        // the saved-key rule alone would pass both.
        let azure = biorouter::providers::azure::AzureProvider::metadata();
        for key in [None, Some(""), Some("  ")] {
            let mut values = vec![
                ("AZURE_OPENAI_ENDPOINT", "https://attacker.example"),
                ("AZURE_OPENAI_DEPLOYMENT_NAME", "gpt"),
            ];
            values.extend(key.map(|key| ("AZURE_OPENAI_API_KEY", key)));
            assert_eq!(
                check(&azure, &values, NoKeyInstalled, &[])
                    .map_err(|r| r.0)
                    .err(),
                Some(StatusCode::FORBIDDEN),
                "key {key:?}"
            );
        }
        assert!(check(
            &azure,
            &[
                ("AZURE_OPENAI_ENDPOINT", "https://my.openai.azure.com"),
                ("AZURE_OPENAI_DEPLOYMENT_NAME", "gpt"),
                ("AZURE_OPENAI_API_KEY", "typed"),
            ],
            NoKeyInstalled,
            &[]
        )
        .is_ok());

        // A provider with no key to type signs in with this computer's own
        // credentials, so a setting it names is refused outright.
        let vertex = biorouter::providers::gcpvertexai::GcpVertexAIProvider::metadata();
        assert_eq!(
            check(
                &vertex,
                &[("GCP_LOCATION", "attacker.example#")],
                NoKeyInstalled,
                &[]
            )
            .map_err(|r| r.0)
            .err(),
            Some(StatusCode::FORBIDDEN)
        );
    }

    /// Review of W2-PRV-2, round 3. Where no person can be proven, EVERY live
    /// check is run only with what the caller typed, not only one whose
    /// candidate names a setting. Without it a live check with no candidate
    /// sent the saved key to whatever host the configuration named.
    #[test]
    fn an_unproven_live_check_sends_only_what_it_typed() {
        use biorouter::providers::base::Provider;
        use biorouter_server::auth::UserActionProof::{NoKeyInstalled, Proven};
        let openai = biorouter::providers::openai::OpenAiProvider::metadata();
        let run = |metadata: &ProviderMetadata,
                   candidate: Option<&[(&str, &str)]>,
                   live: bool,
                   proof,
                   saved: &[&str]| {
            let saved: Vec<String> = saved.iter().map(|key| key.to_string()).collect();
            check_overrides(
                Some(metadata),
                &metadata.name,
                candidate.map(|values| {
                    values
                        .iter()
                        .map(|(key, value)| (key.to_string(), value.to_string()))
                        .collect()
                }),
                live,
                &proof,
                |key| saved.iter().any(|saved| saved == key),
            )
        };

        // No candidate, the key saved: refused, and the sentence names it.
        let (status, sentence) = run(&openai, None, true, NoKeyInstalled, &["OPENAI_API_KEY"])
            .expect_err("a live check does not send the saved key");
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(sentence.contains("the saved OPENAI_API_KEY"), "{sentence}");
        assert!(!sentence.contains("new OpenAI setting"), "{sentence}");
        // No candidate and nothing saved, or a key typed empty: refused, since
        // the check would sign in with nothing the caller typed.
        for candidate in [None, Some(&[("OPENAI_API_KEY", "  ")][..])] {
            let (status, sentence) = run(&openai, candidate, true, NoKeyInstalled, &[])
                .expect_err("a live check needs a typed key");
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert!(
                sentence.contains("needs OPENAI_API_KEY typed in"),
                "{sentence}"
            );
        }
        // A key typed: checked, with every secret it left out empty.
        let overrides = run(
            &openai,
            Some(&[("OPENAI_API_KEY", "sk-typed")]),
            true,
            NoKeyInstalled,
            &["OPENAI_CUSTOM_HEADERS"],
        )
        .expect("a typed key is checked");
        assert_eq!(overrides["OPENAI_CUSTOM_HEADERS"], EMPTY_SECRET_OVERRIDE);

        // A plain construction check stays open, and a person who proved it
        // asked is checked with the saved values.
        assert!(
            run(&openai, None, false, NoKeyInstalled, &["OPENAI_API_KEY"])
                .unwrap()
                .is_empty()
        );
        assert!(run(&openai, None, true, Proven, &["OPENAI_API_KEY"])
            .unwrap()
            .is_empty());

        // Azure signs in with this computer's Entra login when no key is set,
        // so a live check of it with no key typed is refused too.
        let azure = biorouter::providers::azure::AzureProvider::metadata();
        let (status, sentence) = run(&azure, None, true, NoKeyInstalled, &[])
            .expect_err("Azure's own sign-in does not travel either");
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(sentence.contains("AZURE_OPENAI_API_KEY"), "{sentence}");

        // A provider that declares no secret makes no authenticated call, so a
        // live check of its saved settings sends nothing and stays open.
        let ollama = biorouter::providers::ollama::OllamaProvider::metadata();
        assert!(run(&ollama, None, true, NoKeyInstalled, &[])
            .unwrap()
            .is_empty());
    }

    /// Review of W2-PRV-2, round 3, the root cause. A caller holding only the
    /// daemon secret could move a provider's host with `/config/upsert` and
    /// then have a chat, the models route or a live check send the saved key
    /// there. Changing a key that decides where requests go now takes the proof
    /// of a person; re-saving what it already resolves to does not.
    #[tokio::test]
    async fn an_unproven_caller_cannot_move_where_a_provider_sends_its_key() {
        for key in ["OPENAI_HOST", "VERSA_AZURE_ENDPOINT", "DATABRICKS_HOST"] {
            assert!(
                std::env::var(key).is_err(),
                "{key} is set in this test's environment, which decides what it resolves to; \
                 unset it to run this test"
            );
        }
        let dir = tempfile::TempDir::new().unwrap();
        let config = Config::new_with_file_secrets(
            dir.path().join("config.yaml"),
            dir.path().join("secrets.yaml"),
        )
        .unwrap();
        // No proof: an empty header map is never a proven person, whether or
        // not another test in this binary installed a user-action key.
        async fn gate(
            config: &Config,
            key: &str,
            change: DestinationChange<'_>,
        ) -> Option<StatusCode> {
            destination_change_refusal(config, key, change, &HeaderMap::new())
                .await
                .map(|(status, _sentence)| status)
        }
        let refused = |key: &'static str, change| gate(&config, key, change);
        let attacker = Value::from("https://attacker.example");
        let default = Value::from("https://api.openai.com");
        let gateway = Value::from("https://gateway.example");
        let (attacker, default, gateway) = (&attacker, &default, &gateway);

        // Nothing stored: moving it is refused, re-saving the default is not.
        assert_eq!(
            refused("OPENAI_HOST", DestinationChange::Write(attacker)).await,
            Some(StatusCode::CONFLICT)
        );
        assert_eq!(
            refused("openai_host", DestinationChange::Write(attacker)).await,
            Some(StatusCode::CONFLICT),
            "a lower-case spelling is a destination key too"
        );
        assert_eq!(
            refused("OPENAI_HOST", DestinationChange::Write(default)).await,
            None
        );
        // An endpoint no provider declares has no default to re-save.
        assert_eq!(
            refused("VERSA_AZURE_ENDPOINT", DestinationChange::Write(attacker)).await,
            Some(StatusCode::CONFLICT)
        );
        // A value compares as text, however the form typed it: a numeric-default
        // key arrives as a number, and an older save left it quoted.
        assert_eq!(
            setting_text(&Value::from(11543)),
            setting_text(&Value::from("11543"))
        );
        // Ollama sends no credential; its host is a privacy capability key,
        // governed with the privacy master switch, not by this gate.
        assert_eq!(
            gate(&config, "OLLAMA_HOST", DestinationChange::Write(attacker)).await,
            None
        );

        // The user's own host: re-saving it is not a move, changing it is,
        // and so is removing it, which hands requests back to the default.
        config.set_param("OPENAI_HOST", gateway.clone()).unwrap();
        assert_eq!(
            refused("OPENAI_HOST", DestinationChange::Write(gateway)).await,
            None
        );
        for change in [
            DestinationChange::Write(attacker),
            DestinationChange::Write(default),
            DestinationChange::Remove { is_secret: false },
        ] {
            assert_eq!(
                refused("OPENAI_HOST", change).await,
                Some(StatusCode::CONFLICT)
            );
        }
        // Stored as the default, a removal lands where it already goes.
        config.set_param("OPENAI_HOST", default.clone()).unwrap();
        assert_eq!(
            refused(
                "OPENAI_HOST",
                DestinationChange::Remove { is_secret: false }
            )
            .await,
            None
        );
        // Nothing stored where it removes from: nothing moves.
        assert_eq!(
            refused("OPENAI_HOST", DestinationChange::Remove { is_secret: true }).await,
            None
        );
        // A host kept in the secret store (Databricks and Snowflake read one
        // there) is a destination whichever store it is in.
        config
            .set_secret("DATABRICKS_HOST", &Value::from("https://dbc.example"))
            .unwrap();
        assert_eq!(
            refused(
                "DATABRICKS_HOST",
                DestinationChange::Remove { is_secret: true }
            )
            .await,
            Some(StatusCode::CONFLICT)
        );

        // Keys that do not decide where requests go are not this gate's.
        assert_eq!(
            refused("OPENAI_API_KEY", DestinationChange::Write(attacker)).await,
            None
        );
        assert_eq!(
            refused(
                "OPENAI_TIMEOUT",
                DestinationChange::Remove { is_secret: false }
            )
            .await,
            None
        );
    }

    /// Review of W2-PRV-2, round 3. A custom provider keeps its saved key when
    /// an update leaves the key empty, and always keeps its saved headers, so
    /// an unproven update that moves its URL would send them there.
    #[test]
    fn an_unproven_custom_provider_update_cannot_move_its_saved_key() {
        use biorouter::config::declarative_providers::DeclarativeProviderConfig;
        let saved = |headers: Option<HashMap<String, String>>, is_editable| LoadedProvider {
            config: serde_json::from_value::<DeclarativeProviderConfig>(serde_json::json!({
                "name": "custom_lab",
                "engine": "openai",
                "display_name": "Lab gateway",
                "api_key_env": "CUSTOM_LAB_API_KEY",
                "base_url": "https://lab.example/v1",
                "models": [],
                "headers": headers,
            }))
            .unwrap(),
            is_editable,
        };
        let update = |api_url: &str, api_key: &str| UpdateCustomProviderRequest {
            engine: "openai_compatible".to_string(),
            display_name: "Lab gateway".to_string(),
            api_url: api_url.to_string(),
            api_key: api_key.to_string(),
            models: Vec::new(),
            supports_streaming: None,
            headers: None,
        };
        let plain = saved(None, true);
        let agent = RefusalReader::Agent;
        let status = |saved: &LoadedProvider, request| {
            custom_provider_move_refusal(Some(saved), &request, agent).map(|(status, _)| status)
        };

        // Moved with the saved key kept: refused, and the sentence says so.
        let (code, sentence) = custom_provider_move_refusal(
            Some(&plain),
            &update("https://attacker.example", ""),
            agent,
        )
        .expect("the saved key does not move with the URL");
        assert_eq!(code, StatusCode::CONFLICT);
        assert!(sentence.contains("its saved key"), "{sentence}");
        // Moved with a new key typed: that key replaces the saved one.
        assert_eq!(
            status(&plain, update("https://new.example/v1", "sk-typed")),
            None
        );
        // Not moved: renaming or re-listing models moves nothing.
        assert_eq!(status(&plain, update("https://lab.example/v1", "")), None);
        // Saved headers travel with every request, so a typed key is not
        // enough to move them.
        let with_headers = saved(
            Some(HashMap::from([(
                "X-Lab-Token".to_string(),
                "t".to_string(),
            )])),
            true,
        );
        assert_eq!(
            status(&with_headers, update("https://new.example/v1", "sk-typed")),
            Some(StatusCode::CONFLICT)
        );
        // A provider that is not editable keeps its URL whatever is asked.
        assert_eq!(
            status(&saved(None, false), update("https://attacker.example", "")),
            None
        );
        // One that cannot be loaded is reported by the update itself.
        assert!(
            custom_provider_move_refusal(None, &update("https://attacker.example", ""), agent)
                .is_none()
        );

        // A person in a browser is told what works there: typing the key
        // again, when no saved headers would go along, or the desktop app on
        // the computer running Biorouter. Not "this request carried no proof".
        let (_, keyless) = custom_provider_move_refusal(
            Some(&plain),
            &update("https://attacker.example", ""),
            RefusalReader::HostComputer,
        )
        .unwrap();
        assert!(keyless.contains("type the key again"), "{keyless}");
        assert!(keyless.contains("computer running Biorouter"), "{keyless}");
        assert!(!keyless.contains("no proof"), "{keyless}");
        let (_, keyless) = custom_provider_move_refusal(
            Some(&with_headers),
            &update("https://new.example/v1", "sk-typed"),
            RefusalReader::HostComputer,
        )
        .unwrap();
        assert!(
            !keyless.contains("type the key again"),
            "a typed key does not get saved headers through: {keyless}"
        );
        let (_, faulted) = custom_provider_move_refusal(
            Some(&plain),
            &update("https://attacker.example", ""),
            RefusalReader::DesktopWithoutItsKey,
        )
        .unwrap();
        assert!(faulted.contains("Quit and reopen Biorouter"), "{faulted}");
    }

    /// Review of W2-PRV-2, round 4. A daemon that holds no user-action key
    /// refuses every change to a destination key, and on `biorouter serve` the
    /// reader is a person in the Biorouter page of a browser. The agent's
    /// sentence ("this request carried no proof it came from them ... change it
    /// in the provider's settings in the Biorouter app") pointed that person
    /// back at the page they were on. Each reader now gets its own sentence;
    /// the gate does not change.
    #[test]
    fn a_destination_refusal_is_worded_for_whoever_reads_it() {
        let agent = destination_refusal("OPENAI_HOST", RefusalReader::Agent);
        assert!(agent.contains("no proof it came from them"), "{agent}");
        assert!(agent.contains("The user can change it"), "{agent}");

        let person = destination_refusal("OPENAI_HOST", RefusalReader::HostComputer);
        assert!(person.contains("'OPENAI_HOST'"), "{person}");
        assert!(person.contains("`biorouter configure`"), "{person}");
        assert!(person.contains("computer running Biorouter"), "{person}");
        assert!(person.contains("`biorouter serve`"), "{person}");
        for agent_words in ["no proof", "the user's decision", "provider's settings"] {
            assert!(!person.contains(agent_words), "{person}");
        }

        let desktop = destination_refusal("OPENAI_HOST", RefusalReader::DesktopWithoutItsKey);
        assert!(desktop.contains("Quit and reopen Biorouter"), "{desktop}");
        assert!(!desktop.contains("no proof"), "{desktop}");

        for sentence in [agent, person, desktop] {
            assert!(
                sentence.contains("Nothing was changed")
                    || sentence.contains("nothing was changed")
            );
        }
    }

    /// Review of W2-PRV-2, round 4. Amazon Bedrock and SageMaker take their
    /// endpoint from `AWS_ENDPOINT_URL_BEDROCK_RUNTIME`,
    /// `AWS_ENDPOINT_URL_SAGEMAKER_RUNTIME` or `AWS_ENDPOINT_URL`, read from
    /// `config.yaml` or the secret store by `aws_stored_settings` rather than
    /// through `get_param`. None of the three was a destination key, so an
    /// unproven `/config/upsert` could aim the next Bedrock chat, with a stored
    /// Bedrock API key as its bearer token, at any host.
    #[tokio::test]
    async fn an_unproven_caller_cannot_move_the_aws_endpoint_in_either_store() {
        let keys = [
            "AWS_ENDPOINT_URL_BEDROCK_RUNTIME",
            "AWS_ENDPOINT_URL_SAGEMAKER_RUNTIME",
            "AWS_ENDPOINT_URL",
            // A service Biorouter does not call yet: the SDK honours it all the
            // same, and the prefix covers it.
            "AWS_ENDPOINT_URL_STS",
        ];
        for key in keys {
            assert!(
                std::env::var(key).is_err(),
                "{key} is set in this test's environment, which decides what it resolves to; \
                 unset it to run this test"
            );
        }
        let dir = tempfile::TempDir::new().unwrap();
        let config = Config::new_with_file_secrets(
            dir.path().join("config.yaml"),
            dir.path().join("secrets.yaml"),
        )
        .unwrap();
        let store = &config;
        let refused = move |key: &'static str, change: DestinationChange<'static>| async move {
            destination_change_refusal(store, key, change, &HeaderMap::new())
                .await
                .map(|(status, _sentence)| status)
        };
        static ATTACKER: std::sync::LazyLock<Value> =
            std::sync::LazyLock::new(|| Value::from("https://attacker.example"));
        static VPC: std::sync::LazyLock<Value> =
            std::sync::LazyLock::new(|| Value::from("https://vpce-1.bedrock.example"));
        let conflict = Some(StatusCode::CONFLICT);

        // Nothing stored and no provider declares a default: every write is a
        // move, whichever store it is meant for (the gate runs before either).
        for key in keys {
            assert_eq!(
                refused(key, DestinationChange::Write(&ATTACKER)).await,
                conflict,
                "{key}"
            );
        }
        assert_eq!(
            refused(
                "aws_endpoint_url_bedrock_runtime",
                DestinationChange::Write(&ATTACKER)
            )
            .await,
            conflict,
            "a lower-case spelling is the same key"
        );

        // The user's own endpoint in config.yaml: re-saving it moves nothing,
        // changing or removing it does.
        config
            .set_param("AWS_ENDPOINT_URL_BEDROCK_RUNTIME", VPC.clone())
            .unwrap();
        assert_eq!(
            refused(
                "AWS_ENDPOINT_URL_BEDROCK_RUNTIME",
                DestinationChange::Write(&VPC)
            )
            .await,
            None
        );
        for change in [
            DestinationChange::Write(&ATTACKER),
            DestinationChange::Remove { is_secret: false },
        ] {
            assert_eq!(
                refused("AWS_ENDPOINT_URL_BEDROCK_RUNTIME", change).await,
                conflict
            );
        }

        // The same endpoint in the secret store, which `aws_stored_settings`
        // reads too, and where it wins over config.yaml.
        config
            .set_secret("AWS_ENDPOINT_URL", &Value::from("https://vpce-2.example"))
            .unwrap();
        assert_eq!(
            refused(
                "AWS_ENDPOINT_URL",
                DestinationChange::Remove { is_secret: true }
            )
            .await,
            conflict
        );
        assert_eq!(
            refused("AWS_ENDPOINT_URL", DestinationChange::Write(&ATTACKER)).await,
            conflict
        );

        // The credentials themselves are not this gate's.
        assert_eq!(
            refused(
                "AWS_BEARER_TOKEN_BEDROCK",
                DestinationChange::Write(&ATTACKER)
            )
            .await,
            None
        );
    }

    /// Review of W2-PRV-2, round 4. The tests above call the gates directly,
    /// so they keep passing when a route stops calling one. This fails then.
    /// A source scan, as `auth.rs`'s `all_five_raise_channels_call_the_guard`
    /// is: it also asks that the gate come before the write it guards.
    #[test]
    fn the_routes_call_the_destination_gates_before_they_write() {
        let source = include_str!("config_management.rs");
        // The handler's code, without its comments: a comment that names a
        // write (`config.set(.., is_secret)`) is not one.
        let body_of = |signature: &str| -> String {
            let (_, body) = source
                .split_once(signature)
                .unwrap_or_else(|| panic!("{signature} is in this file"));
            let (body, _) = body.split_once("\n}\n").expect("the function's end");
            body.lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        for (handler, gate, writes) in [
            (
                "pub async fn upsert_config(",
                "destination_change_refusal(",
                &["config.set(", "master_switch::write_for("][..],
            ),
            (
                "pub async fn remove_config(",
                "destination_change_refusal(",
                &["config.delete_secret(", "config.delete("][..],
            ),
            (
                "pub async fn update_custom_provider(",
                "custom_provider_move_refusal(",
                &["declarative_providers::update_custom_provider("][..],
            ),
        ] {
            let body = body_of(handler);
            let gated_at = body
                .find(gate)
                .unwrap_or_else(|| panic!("{handler} no longer calls {gate}"));
            for write in writes {
                let written_at = body
                    .find(write)
                    .unwrap_or_else(|| panic!("{handler} no longer writes with {write}"));
                assert!(
                    gated_at < written_at,
                    "{handler} writes with {write} before it calls {gate}"
                );
            }
        }
        // Not vacuous: a handler with no destination gate comes back without one.
        assert!(!body_of("pub async fn read_all_config(").contains("destination_change_refusal("));
    }

    /// Review of W2-PRV-2, round 2. The key a provider is built with under a
    /// candidate is exactly the key typed, whichever of `get_secret` and
    /// `get_secrets` it reads it through. OpenAI and LiteLLM read theirs through
    /// `get_secrets`, which returned the JSON literal with its quotes, so a
    /// correct key went out as `Bearer "sk-..."` and was refused. An all-digit
    /// key is still a string on both paths.
    #[tokio::test]
    async fn a_typed_key_reaches_the_provider_exactly_as_typed() {
        use wiremock::matchers::any;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        async fn sent_under(provider: &str, values: &[(&str, &str)]) -> Vec<wiremock::Request> {
            let server = MockServer::start().await;
            Mock::given(any())
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({ "data": [{ "id": "m" }] })),
                )
                .mount(&server)
                .await;
            let metadata = get_providers()
                .await
                .into_iter()
                .map(|(metadata, _)| metadata)
                .find(|metadata| metadata.name == provider)
                .expect("a registered provider");
            let host_key = metadata
                .config_keys
                .iter()
                .find(|key| key.name.ends_with("_HOST"))
                .expect("a host setting")
                .name
                .clone();
            let mut values: HashMap<String, String> = values
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect();
            values.insert(host_key, server.uri());
            // The route's own composition, as a `biorouter serve` browser meets
            // it: the candidate, then the no-key narrowing, nothing saved.
            let overrides = check_overrides(
                Some(&metadata),
                provider,
                Some(values),
                true,
                &biorouter_server::auth::UserActionProof::NoKeyInstalled,
                |_| false,
            )
            .expect("a candidate that carries its key");
            biorouter::config::with_config_overrides(overrides, async {
                let built = create_with_default_model(provider)
                    .await
                    .expect("the provider builds from the candidate");
                let _ = built.fetch_supported_models().await;
            })
            .await;
            server.received_requests().await.unwrap_or_default()
        }

        fn header(request: &wiremock::Request, name: &str) -> Option<String> {
            request
                .headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        }

        for key in ["sk-typed-123", "1234567890"] {
            let sent = sent_under("openai", &[("OPENAI_API_KEY", key)]).await;
            assert!(!sent.is_empty(), "OpenAI made no call");
            for request in &sent {
                assert_eq!(
                    header(request, "authorization"),
                    Some(format!("Bearer {key}"))
                );
            }

            let sent = sent_under(
                "litellm",
                &[
                    ("LITELLM_API_KEY", key),
                    ("LITELLM_CUSTOM_HEADERS", "X-Team: blue"),
                ],
            )
            .await;
            assert!(!sent.is_empty(), "LiteLLM made no call");
            for request in &sent {
                assert_eq!(
                    header(request, "authorization"),
                    Some(format!("Bearer {key}"))
                );
                assert_eq!(header(request, "x-team"), Some("blue".to_string()));
            }

            // Anthropic reads its key through `get_secret`.
            let sent = sent_under("anthropic", &[("ANTHROPIC_API_KEY", key)]).await;
            assert!(!sent.is_empty(), "Anthropic made no call");
            for request in &sent {
                assert_eq!(header(request, "x-api-key"), Some(key.to_string()));
            }
        }
    }

    /// A stand-in whose model listing answers what each test needs.
    struct Listing(fn() -> Result<Option<Vec<String>>, ProviderError>);

    #[async_trait::async_trait]
    impl biorouter::providers::base::Provider for Listing {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::empty()
        }
        fn get_name(&self) -> &str {
            "listing"
        }
        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new("test-model").unwrap()
        }
        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[biorouter::conversation::message::Message],
            _tools: &[rmcp::model::Tool],
        ) -> Result<
            (
                biorouter::conversation::message::Message,
                biorouter::providers::base::ProviderUsage,
            ),
            ProviderError,
        > {
            Err(ProviderError::ExecutionError("not used".to_string()))
        }
        async fn fetch_supported_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
            if (self.0)()
                .is_err_and(|e| matches!(e, ProviderError::ServerError(ref m) if m == "hang"))
            {
                std::future::pending::<()>().await;
            }
            (self.0)()
        }
    }

    /// W2-PRV-2. Only the provider's rejection of the credentials refuses the
    /// check, with its own message; anything that says nothing about the key
    /// (no listing, a server error, no answer in time) passes, as before.
    #[tokio::test]
    async fn only_a_rejected_credential_fails_the_live_check() {
        let refused = live_credential_refusal(
            "Anthropic",
            &Listing(|| {
                Err(ProviderError::Authentication(
                    "Authentication failed. Status: 401. Response: invalid x-api-key".to_string(),
                ))
            }),
        )
        .await
        .expect("a rejected key fails the check");
        assert!(
            refused.starts_with("Anthropic rejected these credentials:"),
            "{refused}"
        );
        assert!(refused.contains("invalid x-api-key"), "{refused}");

        for accepts in [
            (|| Ok(Some(vec!["m".to_string()]))) as fn() -> _,
            || Ok(None),
            || Err(ProviderError::ServerError("503".to_string())),
            || Err(ProviderError::ServerError("hang".to_string())),
        ] {
            assert_eq!(
                live_credential_refusal_within(
                    "Anthropic",
                    &Listing(accepts),
                    std::time::Duration::from_millis(200)
                )
                .await,
                None
            );
        }
    }

    /// A provider with no model listing that can still be asked, as the Versa
    /// gateways can.
    struct Unlisted;

    #[async_trait::async_trait]
    impl biorouter::providers::base::Provider for Unlisted {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::empty()
        }
        fn get_name(&self) -> &str {
            "unlisted"
        }
        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new("test-model").unwrap()
        }
        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[biorouter::conversation::message::Message],
            _tools: &[rmcp::model::Tool],
        ) -> Result<
            (
                biorouter::conversation::message::Message,
                biorouter::providers::base::ProviderUsage,
            ),
            ProviderError,
        > {
            Err(ProviderError::ExecutionError("not used".to_string()))
        }
        async fn check_credentials(&self) -> Result<(), ProviderError> {
            Err(ProviderError::Authentication(
                "Invalid client id or secret".to_string(),
            ))
        }
    }

    /// T3-SH-3. The live check asked only for a model listing, and a provider
    /// with none (Versa) answered `Ok(None)` without sending anything, so a
    /// wrong Versa key was saved. The check is the provider's own now.
    #[tokio::test]
    async fn the_live_check_asks_a_provider_with_no_listing_its_own_way() {
        use biorouter::providers::base::Provider;
        assert!(matches!(Unlisted.fetch_supported_models().await, Ok(None)));
        let refused = live_credential_refusal("Versa API Azure", &Unlisted)
            .await
            .expect("a key the gateway refuses fails the check");
        assert_eq!(
            refused,
            "Versa API Azure rejected these credentials: Invalid client id or secret"
        );
    }

    /// W2-PRV-8. Privacy is a section of Settings > App, not a tab of its own, so
    /// a refusal that sends the person to "Settings > Privacy" names a place
    /// that does not exist.
    #[test]
    fn the_master_switch_refusal_names_the_real_place() {
        let refusal = master_switch_refusal("BIOROUTER_PRIVACY_TIERS");
        assert!(refusal.contains("Settings > App > Privacy"), "{refusal}");
        assert!(!refusal.contains("Settings > Privacy"), "{refusal}");
    }

    /// W2-PRV-10. Settings saved `0` when its Max turns field was cleared, and
    /// a stored 0 stopped every new chat before its first model call. The write
    /// path refuses anything that is not a whole number of at least 1, with a
    /// sentence, and leaves every other key alone.
    #[test]
    fn max_turns_below_one_is_refused_with_a_sentence() {
        use serde_json::json;
        for refused in [
            json!(0),
            json!(-5),
            json!(""),
            json!("0"),
            json!(" -1 "),
            json!(2.5),
            json!("many"),
            json!(null),
            json!(u64::from(u32::MAX) + 1),
        ] {
            let refusal = config_value_refusal("BIOROUTER_MAX_TURNS", &refused)
                .unwrap_or_else(|| panic!("{refused} was accepted as a max-turns limit"));
            assert!(
                refusal.starts_with("Max turns must be a whole number of at least 1"),
                "{refusal}"
            );
        }
        for accepted in [json!(1), json!(100), json!("250"), json!(u32::MAX)] {
            assert_eq!(
                config_value_refusal("BIOROUTER_MAX_TURNS", &accepted),
                None,
                "{accepted}"
            );
        }
        // Only the one key is shaped here.
        assert_eq!(config_value_refusal("SOME_OTHER_KEY", &json!(0)), None);
    }

    /// T3-SH-7: Edit configuration saved any provider name, and every new chat
    /// then failed to start.
    #[test]
    fn a_provider_name_must_be_one_this_daemon_can_build() {
        use serde_json::json;
        let registered = ["openai".to_string(), "versa_azure".to_string()];
        for key in [
            "BIOROUTER_PROVIDER",
            "BIOROUTER_LEAD_PROVIDER",
            "biorouter_provider",
        ] {
            assert!(names_a_provider(key), "{key}");
            for accepted in [json!("openai"), json!(" versa_azure ")] {
                assert_eq!(
                    unknown_provider_refusal(key, &accepted, &registered),
                    None,
                    "{key} = {accepted}"
                );
            }
            for refused in [
                json!("bogus_provider_qa"),
                json!("OpenAI"),
                json!(""),
                json!(null),
                json!(3),
            ] {
                let refusal = unknown_provider_refusal(key, &refused, &registered)
                    .unwrap_or_else(|| panic!("{key} = {refused} was accepted"));
                assert!(
                    refusal.contains("is not a provider Biorouter can use"),
                    "{refusal}"
                );
                assert!(refusal.contains(key), "{refusal}");
            }
        }
        let refusal = unknown_provider_refusal(
            "BIOROUTER_PROVIDER",
            &json!("bogus_provider_qa"),
            &registered,
        )
        .expect("refused");
        assert!(
            refusal.starts_with("'bogus_provider_qa' is not a provider"),
            "{refusal}"
        );
        // Only the keys that name a provider are checked.
        assert!(!names_a_provider("BIOROUTER_MODEL"));
        assert!(!names_a_provider("OPENAI_HOST"));
    }

    /// The route asks the registry before it writes, and after every privacy
    /// gate, so a refused name never reaches the file and an unproven caller is
    /// still told about the proof first.
    #[test]
    fn upsert_refuses_an_unknown_provider_before_it_writes() {
        let source = include_str!("config_management.rs");
        let (_, body) = source
            .split_once("pub async fn upsert_config(")
            .expect("upsert_config");
        let (body, _) = body.split_once("\n}\n").expect("the function's end");
        // Without comments: a comment that names a write is not one.
        let body = body
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let refused_at = body
            .find("unknown_provider_refusal(")
            .expect("upsert_config checks provider names");
        let written_at = body.find("config.set(").expect("upsert_config writes");
        let gated_at = body
            .find("destination_change_refusal(")
            .expect("upsert_config has its destination gate");
        let capability_at = body
            .find("is_capability_key(")
            .expect("upsert_config has its capability gate");
        assert!(refused_at < written_at);
        assert!(capability_at < refused_at && gated_at < refused_at);
    }

    /// `GET /config/providers/{name}/models` is named and documented as the model
    /// list, and for nine builtins it answered `[]`.
    ///
    /// ⚠ The cause is a default, not a failure: `fetch_supported_models` returns
    /// `Ok(None)` unless a provider overrides it, and 9 of the 23 registered
    /// builtins do not override it. Every one of those nine declares a catalog the
    /// settings grid renders, so the route reported "no models" for a provider
    /// whose models the user could see on screen — under a `200 Models fetched
    /// successfully`.
    ///
    /// The nine are named rather than derived, because deriving them needs a live
    /// instance of each: credentials, and a network call for the ones that do
    /// fetch. If one grows a live fetch later it leaves the `Ok(None)` arm and its
    /// row here becomes redundant rather than wrong.
    #[tokio::test]
    async fn a_provider_with_no_live_fetch_reports_the_models_it_declares() {
        let all = biorouter::providers::providers().await;
        let mut checked = 0;
        for name in [
            "azure_openai",
            "aws_bedrock",
            "versa_azure",
            "versa_bedrock",
            "xai",
            "xiaomi_mimo",
            "snowflake",
            "zai",
            "sagemaker_tgi",
        ] {
            let Some((metadata, _)) = all.iter().find(|(m, _)| m.name == name) else {
                // `aws_bedrock`, `versa_bedrock` and `sagemaker_tgi` are behind
                // the `aws-providers` feature.
                continue;
            };
            assert!(
                !metadata.known_models.is_empty(),
                "{name} declares a catalog — that is what made `[]` a false answer"
            );
            assert_eq!(
                declared_model_names(metadata),
                metadata
                    .known_models
                    .iter()
                    .map(|model| model.name.clone())
                    .collect::<Vec<_>>(),
                "{name} must report exactly what it declares, in order"
            );
            checked += 1;
        }
        assert!(checked >= 6, "only {checked} of the nine were reachable");
    }

    /// …and nothing is invented for a provider that declares nothing. `litellm`
    /// and `ollama` are the two builtins with an empty catalog; both have a live
    /// fetch, so neither reaches the arm above — but the projection has to be
    /// faithful in that direction too, or the fix reads as "always non-empty".
    #[tokio::test]
    async fn an_empty_catalog_projects_to_an_empty_list() {
        let all = biorouter::providers::providers().await;
        for name in ["litellm", "ollama"] {
            let Some((metadata, _)) = all.iter().find(|(m, _)| m.name == name) else {
                continue;
            };
            assert!(metadata.known_models.is_empty(), "{name} declares none");
            assert!(declared_model_names(metadata).is_empty(), "{name}");
        }
    }

    /// A recovery that could not write says so in BOTH halves of its answer.
    ///
    /// The route reported plain success for a config it had just failed to
    /// write (finding M9): `POST /config/recover` answered HTTP 200 `"Config
    /// recovery completed. Recovered 23 keys: …"` while the corrupt bytes were
    /// still on disk, because the backup-restore arm of the recovery recorded
    /// nothing. With the record in place the sentence carries the warning, and
    /// `persisted` carries it in a form no caller has to parse prose for.
    #[test]
    fn a_recovery_that_could_not_write_says_so_in_the_sentence_and_in_the_flag() {
        let report = recovery_report(
            vec![
                "BIOROUTER_MODEL".to_string(),
                "BIOROUTER_PROVIDER".to_string(),
            ],
            Some(ConfigWriteFailure::ValuesInMemoryOnly(
                "Config file I/O failed: Permission denied (os error 13)".to_string(),
            )),
        );

        assert!(
            !report.persisted,
            "the values reached memory and nothing else"
        );
        assert_eq!(
            report.write_error.as_deref(),
            Some("Config file I/O failed: Permission denied (os error 13)"),
            "the cause travels verbatim, so a caller need not re-derive it from the prose"
        );
        assert!(
            report.message.contains("in memory only"),
            "the person reading this has to be told the recovery did not land; got {:?}",
            report.message
        );
        assert!(
            report.message.contains("config.yaml on disk is unchanged"),
            "and told what is still on disk, which is the half M9 measured as absent; got {:?}",
            report.message
        );
        assert!(
            report.message.contains("Permission denied (os error 13)"),
            "with the reason, so the message is actionable; got {:?}",
            report.message
        );
        assert!(
            report
                .message
                .starts_with("Config recovery completed. Recovered 2 keys:"),
            "without losing what it did recover; got {:?}",
            report.message
        );
    }

    /// A recovery that landed carries no warning at all.
    ///
    /// The other half of the requirement, and the one that is easy to lose: a
    /// note that outlives its cause tells the user their settings are being
    /// lost while they are being saved, which is worse than saying nothing.
    /// The config layer retires a failure once it stops being true (a write
    /// succeeds, or the file loads and a write would land); this pins that the
    /// route says nothing once it has.
    #[test]
    fn a_recovery_that_persisted_carries_no_warning() {
        let report = recovery_report(vec!["BIOROUTER_MODEL".to_string()], None);

        assert!(report.persisted);
        assert_eq!(report.write_error, None);
        assert_eq!(
            report.message, "Config recovery completed. Recovered 1 keys: BIOROUTER_MODEL",
            "no warning, no hedging, and byte-identical to what this route has always said \
             in the case that is fine"
        );
    }

    /// Nothing recoverable AND nothing writable is still two facts, not one.
    ///
    /// The empty-keys arm had its own sentence and its own copy of the suffix,
    /// which is exactly how one of two branches comes to lose a later edit.
    #[test]
    fn a_recovery_with_nothing_to_recover_still_reports_that_it_could_not_write() {
        let report = recovery_report(
            vec![],
            Some(ConfigWriteFailure::ValuesInMemoryOnly(
                "No space left on device".to_string(),
            )),
        );

        assert!(!report.persisted);
        assert!(report.recovered_keys.is_empty());
        assert!(
            report
                .message
                .starts_with("Config recovery completed, but no data was recoverable."),
            "got {:?}",
            report.message
        );
        assert!(
            report.message.contains("in memory only")
                && report.message.contains("No space left on device"),
            "got {:?}",
            report.message
        );
    }

    /// The flag and the error are one fact, and cannot disagree — for either
    /// shape a failure can take.
    #[test]
    fn persisted_is_exactly_the_absence_of_a_write_error() {
        for failure in [
            None,
            Some(ConfigWriteFailure::ValuesInMemoryOnly(
                "any failure at all".to_string(),
            )),
            Some(ConfigWriteFailure::NotWritable(
                "any failure at all".to_string(),
            )),
        ] {
            let expected = failure.is_none();
            let report = recovery_report(vec!["K".to_string()], failure);
            assert_eq!(
                report.persisted, expected,
                "a report may never claim to have persisted while carrying the error that \
                 says it did not, nor the reverse"
            );
            assert_eq!(report.write_error.is_none(), expected);
        }
    }

    /// A config that loads but cannot be written is NOT "in memory only".
    ///
    /// The file loads, so the values in use are the ones on disk and the next
    /// start will not recover anything — M9's sentence would be false in three
    /// clauses out of four. What is true is narrower: a setting changed now
    /// will not be saved. Reached when a corrupt config is repaired but its
    /// directory is left unwritable; before F2's fix this state carried the
    /// stale M9 note instead.
    #[test]
    fn a_config_that_loads_but_cannot_be_written_is_not_called_in_memory_only() {
        let report = recovery_report(
            vec!["BIOROUTER_MODEL".to_string()],
            Some(ConfigWriteFailure::NotWritable(
                "Config file I/O failed: Permission denied (os error 13)".to_string(),
            )),
        );

        assert!(!report.persisted, "a change made now will not be saved");
        assert_eq!(
            report.write_error.as_deref(),
            Some("Config file I/O failed: Permission denied (os error 13)")
        );
        assert_eq!(
            report.message,
            "Config recovery completed. Recovered 1 keys: BIOROUTER_MODEL ⚠ config.yaml loads, \
             but it cannot be written right now (Config file I/O failed: Permission denied (os \
             error 13)), so a setting changed in this session will not be saved."
        );
        for false_here in ["in memory only", "on disk is unchanged", "recover again"] {
            assert!(
                !report.message.contains(false_here),
                "{false_here:?} is not true of a config that loads; got {:?}",
                report.message
            );
        }
    }

    /// Finding F2: once the config is healed, recovery stops warning.
    ///
    /// Measured on `7c96d796`: after one genuine write failure the permissions
    /// were restored and the file repaired, and three consecutive `POST
    /// /config/recover` calls on the healthy, writable, valid config all
    /// answered `persisted: false` with the stale `Permission denied` — a
    /// config that loads needs no recovery, so the reload wrote nothing, and a
    /// write was the only thing that cleared the record. Three calls here
    /// because three is what was measured.
    ///
    /// Portable: the config's parent is a FILE, which fails the write the same
    /// way on every platform. `each_recovery_describes_the_config_as_it_is_now`
    /// below is the literal `chmod` sequence.
    #[test]
    fn a_recovery_after_the_config_was_healed_reports_persisted_with_no_note() {
        let dir = tempfile::TempDir::new().unwrap();
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, "not a directory").unwrap();
        let config_path = blocked.join("config.yaml");
        let config =
            Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml")).unwrap();

        let refused =
            run_recovery(&config).expect("an unwritable config still recovers into memory");
        assert!(
            !refused.persisted && refused.write_error.is_some(),
            "the premise: the first recovery could not write, and said so; got {:?}",
            refused.message
        );

        // Healed from outside this process: writable, and holding a valid config.
        std::fs::remove_file(&blocked).unwrap();
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(&config_path, "BIOROUTER_MODEL: gpt-5.5\n").unwrap();

        for call in 1..=3 {
            let report = run_recovery(&config).expect("a healthy config recovers");
            assert!(
                report.persisted,
                "call {call}: the config loads and can be written, so the recovery persisted; \
                 got {:?}",
                report.message
            );
            assert_eq!(report.write_error, None, "call {call}");
            assert_eq!(
                report.message, "Config recovery completed. Recovered 1 keys: BIOROUTER_MODEL",
                "call {call}: no note, byte-identical to the healthy answer"
            );
        }
    }

    /// The F2 measurement, step for step, with the step between the two ends
    /// that neither the finding nor #217 measured.
    ///
    /// 1. A corrupt `config.yaml` beside a usable `.bak`, the file `0o444` and
    ///    the directory `0o555`: the recovery cannot write what it recovered
    ///    (M9, fixed by #217).
    /// 2. The file repaired, the directory still `0o555`: the config loads, so
    ///    the values are the file's, but a change still cannot be saved. Both
    ///    halves of that have to be said, and "in memory only" would be false.
    /// 3. The directory restored: nothing to warn about, three times over.
    ///
    /// unix-only because a mode is how the finding made the directory
    /// unwritable; the portable assertion of step 3 is the test above.
    #[cfg(unix)]
    #[test]
    fn each_recovery_describes_the_config_as_it_is_now() {
        use std::os::unix::fs::PermissionsExt;

        /// A `TempDir` still at `0o555` cannot delete its own contents, so the
        /// modes are restored whatever happens.
        struct RestoreModes(std::path::PathBuf, std::path::PathBuf);
        impl Drop for RestoreModes {
            fn drop(&mut self) {
                let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
                let _ = std::fs::set_permissions(&self.1, std::fs::Permissions::from_mode(0o644));
            }
        }
        let mode = |path: &std::path::Path, bits: u32| {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(bits)).unwrap()
        };

        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("config.yaml");
        // The exact 27 bytes of the measurement.
        std::fs::write(&config_path, "BIOROUTER_MODEL: [unclosed\n").unwrap();
        std::fs::write(
            dir.path().join("config.yaml.bak"),
            "BIOROUTER_MODEL: gpt-5.5\n",
        )
        .unwrap();
        let config =
            Config::new_with_file_secrets(&config_path, dir.path().join("secrets.yaml")).unwrap();

        let _restore = RestoreModes(dir.path().to_path_buf(), config_path.clone());
        mode(&config_path, 0o444);
        mode(dir.path(), 0o555);
        // Root ignores the mode, and then every premise below is false.
        if std::fs::write(dir.path().join("writability-probe"), "x").is_ok() {
            eprintln!("skipped: this process can write a 0o555 directory (running as root?)");
            return;
        }

        // 1. M9 — #217's half, re-asserted so the steps after it mean something.
        let unwritable = run_recovery(&config).unwrap();
        assert!(!unwritable.persisted, "{:?}", unwritable.message);
        assert!(
            unwritable.message.contains("in memory only"),
            "the corrupt bytes are still on disk, so the values really are in memory only; \
             got {:?}",
            unwritable.message
        );

        // 2. The file repaired in place; the directory still refuses writes.
        mode(&config_path, 0o644);
        std::fs::write(&config_path, "BIOROUTER_MODEL: gpt-5.5\n").unwrap();
        let read_only = run_recovery(&config).unwrap();
        assert!(
            !read_only.persisted,
            "a change made now still cannot be saved; got {:?}",
            read_only.message
        );
        assert!(
            read_only
                .write_error
                .as_deref()
                .is_some_and(|e| e.contains("Permission denied")),
            "and the reason is the one that holds NOW; got {:?}",
            read_only.write_error
        );
        assert!(
            !read_only.message.contains("in memory only")
                && !read_only.message.contains("recover again"),
            "the file loads and holds these values, so neither \"in memory only\" nor \"the \
             next start will recover again\" is true any more; got {:?}",
            read_only.message
        );
        assert!(
            read_only
                .message
                .contains("config.yaml loads, but it cannot be written right now"),
            "what IS true has to be said instead of nothing; got {:?}",
            read_only.message
        );

        // 3. The directory restored: F2.
        mode(dir.path(), 0o755);
        for call in 1..=3 {
            let healed = run_recovery(&config).unwrap();
            assert!(healed.persisted, "call {call}: got {:?}", healed.message);
            assert_eq!(healed.write_error, None, "call {call}");
            assert_eq!(
                healed.message, "Config recovery completed. Recovered 1 keys: BIOROUTER_MODEL",
                "call {call}"
            );
        }
    }

    #[tokio::test]
    async fn test_read_model_limits() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Secret-Key", "test".parse().unwrap());

        let result = read_config(Json(ConfigKeyQuery {
            key: "model-limits".to_string(),
            is_secret: false,
        }))
        .await;

        assert!(result.is_ok());
        let response = match result.unwrap().0 {
            ConfigValueResponse::Value(value) => value,
            ConfigValueResponse::MaskedValue(_) => panic!("unexpected secret"),
        };

        let limits: Vec<biorouter::model::ModelLimitConfig> =
            serde_json::from_value(response).unwrap();
        assert!(!limits.is_empty());

        let gpt4_limit = limits.iter().find(|l| l.pattern == "gpt-4o");
        assert!(gpt4_limit.is_some());
        assert_eq!(gpt4_limit.unwrap().context_limit, 128_000);
    }

    #[tokio::test]
    async fn detectable_providers_route_lists_known_providers() {
        let Json(resp) = get_detectable_providers().await;
        let names: Vec<&str> = resp.providers.iter().map(|p| p.name.as_str()).collect();
        for expected in [
            "openai",
            "anthropic",
            "google",
            "groq",
            "xai",
            "zai",
            "xiaomi_mimo",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
        // Display names should be resolved from metadata, not left as the id.
        let openai = resp.providers.iter().find(|p| p.name == "openai").unwrap();
        assert!(!openai.display_name.is_empty());
    }

    #[tokio::test]
    async fn pricing_endpoint_uses_shared_resolver_and_exposes_cache_rates() {
        let query = PricingQuery {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
        };
        let expected = resolved_provider_model_pricing(&query.provider, &query.model)
            .await
            .unwrap();

        let Json(response) = get_pricing(Json(query)).await.unwrap();
        let actual = &response.pricing[0];

        assert_eq!(response.source, "resolved");
        assert_eq!(actual.input_token_cost, expected.input_token_cost);
        assert_eq!(actual.output_token_cost, expected.output_token_cost);
        assert_eq!(actual.cache_read_cost, expected.cache_read_cost);
        assert_eq!(actual.cache_write_cost, expected.cache_write_cost);
    }
}

#[cfg(test)]
mod affiliation_wire_tests {
    //! What `GET /config/providers` actually puts on the wire for DR-26's third
    //! axis (issue #56).
    //!
    //! The mapping itself is pinned in `providers::base::affiliation_view_tests`,
    //! where it lives. These assert the two things only this route can get
    //! wrong: which key the field arrives under, and that "no affiliation" is a
    //! rendered `null` rather than a silently missing key — the renderer's
    //! `readProviderAffiliation` reads `row.affiliation`, and a key that moved
    //! would make every badge disappear with nothing failing.

    use super::*;
    use biorouter::providers::base::{ProviderAffiliation, ProviderAffiliationKind};

    fn row(affiliation: Option<ProviderAffiliation>) -> ProviderDetails {
        row_with_tier(affiliation, None)
    }

    fn row_with_tier(
        affiliation: Option<ProviderAffiliation>,
        resolved_tier: Option<ProviderTier>,
    ) -> ProviderDetails {
        ProviderDetails {
            name: "versa_azure".to_string(),
            metadata: ProviderMetadata::empty(),
            is_configured: true,
            provider_type: ProviderType::Builtin,
            affiliation,
            resolved_tier,
            unavailable_reason: None,
        }
    }

    /// The instance-resolved tier travels under `resolved_tier`, beside the
    /// metadata rather than inside it — the same rule as the affiliation, and
    /// for the same reason.
    ///
    /// ⚠ **The key is what the renderer's `readResolvedProviderTier` reads.** A
    /// key that moved, or that serialised as a missing field instead of `null`,
    /// would send every consumer back to "unresolved" — which is fail-safe but
    /// silently restores the defect this field exists to fix: the composer would
    /// stop judging the pairing at all.
    #[test]
    fn the_resolved_tier_travels_beside_the_metadata_under_its_own_key() {
        let json = serde_json::to_value(row_with_tier(None, Some(ProviderTier::Private))).unwrap();
        assert_eq!(json["resolved_tier"], serde_json::json!("private"));
        assert!(
            json["metadata"].get("resolved_tier").is_none(),
            "the instance-resolved tier must not be folded into the type-level metadata"
        );
    }

    /// Unresolved is a rendered `null`, never a missing key — see the module
    /// note above, and `readResolvedProviderTier`, which treats both as
    /// "judge nothing" but only one of which is a contract.
    #[test]
    fn an_unresolved_tier_is_a_rendered_null() {
        let json = serde_json::to_value(row_with_tier(None, None)).unwrap();
        assert_eq!(json["resolved_tier"], serde_json::Value::Null);
    }

    /// The whole point of the field: it is NOT `metadata.tier`.
    ///
    /// `ProviderMetadata::empty()` carries the default tier (Public — "a
    /// provider module that forgets `tier()` gets less reach, never more"),
    /// while this instance resolved Private. A consumer reading the metadata
    /// would call UCSF Versa public, which is the reported defect.
    #[test]
    fn the_resolved_tier_can_disagree_with_the_type_level_one() {
        let json = serde_json::to_value(row_with_tier(None, Some(ProviderTier::Private))).unwrap();
        assert_eq!(json["resolved_tier"], serde_json::json!("private"));
        assert_eq!(json["metadata"]["tier"], serde_json::json!("public"));
    }

    /// ⚠ **Beside the metadata, not inside it.** `ProviderMetadata` is the
    /// type-level claim. Its own `tier` field carries "do not hang a badge on
    /// this field" — and this value is instance-resolved. A renderer that read
    /// `row.metadata.affiliation` would find nothing, so the two must not be
    /// allowed to swap silently.
    #[test]
    fn the_affiliation_travels_beside_the_metadata_not_inside_it() {
        let json = serde_json::to_value(row(Some(ProviderAffiliation {
            kind: ProviderAffiliationKind::Institutions,
            institutions: vec![biorouter::providers::base::AffiliationInstitution {
                id: "ucsf".to_string(),
                display_name: Some("UCSF".to_string()),
            }],
        })))
        .expect("a provider row serialises");

        assert_eq!(json["affiliation"]["kind"], "institutions");
        assert_eq!(json["affiliation"]["institutions"][0]["id"], "ucsf");
        assert_eq!(
            json["affiliation"]["institutions"][0]["display_name"],
            "UCSF"
        );
        assert!(
            json["metadata"].get("affiliation").is_none(),
            "the type-level metadata must not carry an instance-resolved value"
        );
    }

    /// A public provider has no affiliation at all, and the key is present and
    /// `null` rather than absent — the renderer treats both the same, but an
    /// absent key is indistinguishable from a daemon that predates the field,
    /// and this route is the one that knows the difference.
    #[test]
    fn a_provider_with_no_affiliation_serialises_an_explicit_null() {
        let json = serde_json::to_value(row(None)).expect("a provider row serialises");
        assert!(json.as_object().unwrap().contains_key("affiliation"));
        assert!(json["affiliation"].is_null());
    }

    /// A row from a daemon that predates the field still deserialises — the
    /// `#[serde(default)]` that makes the addition non-breaking for any client
    /// or test fixture round-tripping this type.
    #[test]
    fn a_row_without_the_field_still_reads() {
        let parsed: ProviderDetails = serde_json::from_value(serde_json::json!({
            "name": "openai",
            "metadata": serde_json::to_value(ProviderMetadata::empty()).unwrap(),
            "is_configured": false,
            "provider_type": "Builtin",
        }))
        .expect("a row predating the field is still a row");
        assert!(parsed.affiliation.is_none());
    }
}

/// Task 30A (issue #56, DR-17 requirement 3): `GET /privacy/disclosure` and
/// `POST /privacy/disclosure/ack`.
///
/// ⚠ **Handlers, not `oneshot` over a `Router`.** These two routes take no
/// `AppState`, and building one opens the ONE session DB this binary shares
/// (`routes::agent::working_dir_lock_tests`). Calling the handlers directly is
/// how the rest of this file's tests reach `read_config` and
/// `get_detectable_providers`, and it exercises the same guard the router would.
#[cfg(test)]
mod privacy_disclosure_tests {
    use super::*;
    use crate::routes::session::diverge_tests::{
        install_test_user_action_key, TEST_USER_ACTION_KEY,
    };
    use serial_test::serial;

    /// A request holding the daemon secret and, optionally, DR-16's proof of
    /// user. `None` is the caller AR-11/AR-15 establish is indistinguishable
    /// from the model.
    fn headers_with(user_action: Option<&str>) -> http::HeaderMap {
        let mut headers = http::HeaderMap::new();
        headers.insert("X-Secret-Key", "test".parse().unwrap());
        if let Some(key) = user_action {
            headers.insert("X-User-Action", key.parse().unwrap());
        }
        headers
    }

    #[tokio::test]
    #[serial]
    async fn the_acknowledgement_is_recorded_once_and_is_not_agent_writable() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        // Its own config root, or this test writes the acknowledgement into the
        // developer's real `~/.config/biorouter` and every later run of it
        // starts already-acknowledged.
        let dir = tempfile::TempDir::new().unwrap();
        let _env = crate::test_sandbox::relocate_path_root(dir.path());
        install_test_user_action_key();

        // Once per install, not once per session: a dialog on every chat is
        // clicked through, which is exactly the outcome this task exists to
        // avoid.
        assert!(!get_privacy_disclosure().await.0.acknowledged);

        // And it is a USER act. A model that could acknowledge on the user's
        // behalf would silently remove the only thing making DR-17's accepted
        // risks acceptable.
        let refused = ack_privacy_disclosure(headers_with(None))
            .await
            .expect_err("a caller holding only the daemon secret must be refused");
        assert_eq!(refused.0, StatusCode::FORBIDDEN);
        assert!(!get_privacy_disclosure().await.0.acknowledged);

        ack_privacy_disclosure(headers_with(Some(TEST_USER_ACTION_KEY)))
            .await
            .expect("the user's own acknowledgement is recorded");
        assert!(get_privacy_disclosure().await.0.acknowledged);
    }

    #[tokio::test]
    #[serial]
    async fn the_route_serves_the_one_copy_rather_than_a_second_one() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        // The renderer holds no English of its own; this is the wire it gets it
        // over. Compared against the constants themselves, so a second copy
        // written into this handler fails here rather than in a screenshot.
        let dir = tempfile::TempDir::new().unwrap();
        let _env = crate::test_sandbox::relocate_path_root(dir.path());
        let served = get_privacy_disclosure().await.0;
        assert_eq!(served.long, biorouter::privacy::disclosure::COPY_LONG);
        assert_eq!(served.short, biorouter::privacy::disclosure::COPY_SHORT);
        assert_eq!(
            served.settings,
            biorouter::privacy::disclosure::COPY_SETTINGS
        );
        assert_eq!(
            served.settings_title,
            biorouter::privacy::disclosure::COPY_SETTINGS_TITLE
        );
        assert_eq!(
            served.title_template,
            biorouter::privacy::disclosure::COPY_TITLE_TEMPLATE
        );
    }
}

/// F6 of the 2026-09-10 provider QA run, at the route: a coding agent whose CLI
/// is missing is served `is_configured: false` WITH the reason the model picker
/// prints on the row it disables — and an ordinary row carries an explicit
/// `null` in the same key.
///
/// ⚠ Exercised through `provider_details`, the one function `providers()` maps
/// over, rather than through the whole route: `GET /config/providers` builds
/// every configured provider in the developer's real config, which no unit test
/// should do. The command key is pinned through the environment under
/// `env_lock`, so the real config file never decides the outcome.
#[cfg(test)]
mod readiness_wire_tests {
    use super::*;
    use biorouter::providers::base::Provider;
    use biorouter::providers::codex::CodexProvider;
    use biorouter::providers::coding_agent::CodingAgentKind;

    #[tokio::test]
    async fn a_codex_row_whose_cli_is_missing_is_unconfigured_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nonexistent").join("codex");
        let _env = env_lock::lock_env([("CODEX_COMMAND", Some(missing.to_str().unwrap()))]);

        let row = provider_details(CodexProvider::metadata(), ProviderType::Builtin).await;

        assert!(
            !row.is_configured,
            "the badge and the picker both key on this"
        );
        assert_eq!(
            row.unavailable_reason.as_deref(),
            Some(CodingAgentKind::Codex.not_installed_summary().as_str())
        );
        // Nothing was constructed for a provider that cannot be bound.
        assert!(row.resolved_tier.is_none() && row.affiliation.is_none());

        let json = serde_json::to_value(row).unwrap();
        assert_eq!(json["is_configured"], serde_json::json!(false));
        assert_eq!(
            json["unavailable_reason"],
            serde_json::json!(CodingAgentKind::Codex.not_installed_summary())
        );
    }

    /// The control: a codex row whose CLI resolves is configured and carries no
    /// reason — so the test above cannot pass for a route that refuses Codex
    /// outright.
    #[tokio::test]
    async fn a_codex_row_whose_cli_resolves_is_configured_with_no_reason() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("codex");
        std::fs::write(&exe, b"#!/bin/sh\n").unwrap();
        let _env = env_lock::lock_env([("CODEX_COMMAND", Some(exe.to_str().unwrap()))]);

        let row = provider_details(CodexProvider::metadata(), ProviderType::Builtin).await;

        assert!(row.is_configured);
        assert_eq!(row.unavailable_reason, None);
    }

    /// Usable and not-set-up rows alike serve the key as `null`, never omit it:
    /// an absent key is indistinguishable from a daemon that predates the field.
    #[test]
    fn a_row_with_nothing_to_explain_serialises_an_explicit_null() {
        let row = ProviderDetails {
            name: "openai".to_string(),
            metadata: ProviderMetadata::empty(),
            is_configured: false,
            provider_type: ProviderType::Builtin,
            affiliation: None,
            resolved_tier: None,
            unavailable_reason: None,
        };
        let json = serde_json::to_value(row).unwrap();
        assert!(json.as_object().unwrap().contains_key("unavailable_reason"));
        assert!(json["unavailable_reason"].is_null());
    }
}

#[cfg(test)]
mod extension_credential_tests {
    use super::*;
    use crate::routes::session::diverge_tests::{
        install_test_user_action_key, TEST_USER_ACTION_KEY,
    };

    #[tokio::test]
    async fn credential_routes_reject_unproven_and_model_callers_before_accessing_secrets() {
        install_test_user_action_key();
        for proof in [None, Some("spoofed-proof"), Some(TEST_USER_ACTION_KEY)] {
            let mut headers = http::HeaderMap::new();
            if let Some(proof) = proof {
                headers.insert("X-User-Action", proof.parse().unwrap());
            }
            if proof == Some(TEST_USER_ACTION_KEY) {
                headers.insert("X-Caller-Provider", "openai".parse().unwrap());
            }
            assert_eq!(
                get_extension_credentials(headers.clone(), Path("unused".into()))
                    .await
                    .unwrap_err()
                    .0,
                StatusCode::FORBIDDEN
            );
            assert_eq!(
                purge_extension_credentials(
                    headers,
                    Path("unused".into()),
                    Json(PurgeExtensionCredentialsRequest {
                        keys: vec!["ARBITRARY_KEY".into()]
                    })
                )
                .await
                .unwrap_err()
                .0,
                StatusCode::FORBIDDEN
            );
        }
        let mut headers = http::HeaderMap::new();
        headers.insert("X-User-Action", TEST_USER_ACTION_KEY.parse().unwrap());
        assert!(require_credential_user(&headers).is_ok());
    }
}

/// Review of W2-PRV-2, round 4, at the handlers. Each test runs in a process of
/// its own (`test_sandbox::in_a_process_of_its_own`), for two reasons: the
/// user-action digest is a process-global that one test must leave uninstalled
/// and the other must install, and `Config::global()` there is a fresh config
/// under that process's own sandbox root, which a proven write may change.
///
/// ⚠ No secret is written or read through `Config::global()` unless the
/// sandbox keeps secrets in a file: otherwise that store is the developer's
/// real keychain. The gate itself is store-agnostic on a write (it runs before
/// either store is touched), and the secret store's half is pinned against a
/// throwaway `Config` in `an_unproven_caller_cannot_move_the_aws_endpoint_in_either_store`.
#[cfg(test)]
mod destination_route_tests {
    use super::*;
    use crate::routes::session::diverge_tests::{
        install_test_user_action_key, TEST_USER_ACTION_KEY,
    };
    use biorouter_server::auth::{user_action_proof, UserActionProof};
    use serial_test::serial;

    const ENDPOINT: &str = "AWS_ENDPOINT_URL_BEDROCK_RUNTIME";
    const ATTACKER: &str = "https://attacker.example";
    const VPC: &str = "https://vpce-1.bedrock.example";

    fn headers_with(user_action: Option<&str>) -> http::HeaderMap {
        let mut headers = http::HeaderMap::new();
        headers.insert("X-Secret-Key", "test".parse().unwrap());
        if let Some(key) = user_action {
            headers.insert("X-User-Action", key.parse().unwrap());
        }
        headers
    }

    fn write(key: &str, value: &str, is_secret: bool) -> Json<UpsertConfigQuery> {
        Json(UpsertConfigQuery {
            key: key.to_string(),
            value: Value::from(value),
            is_secret,
            confirm: None,
        })
    }

    fn removal(key: &str) -> Json<ConfigKeyQuery> {
        Json(ConfigKeyQuery {
            key: key.to_string(),
            is_secret: false,
        })
    }

    fn stored(key: &str) -> Option<Value> {
        Config::global().all_values().ok()?.get(key).cloned()
    }

    fn preconditions() {
        for key in [ENDPOINT, "AWS_ENDPOINT_URL"] {
            assert!(
                std::env::var(key).is_err(),
                "{key} is set in this test's environment, which decides what it resolves to"
            );
        }
        assert!(
            stored(ENDPOINT).is_none(),
            "the sandbox config starts empty"
        );
    }

    /// `biorouter serve`'s daemon: no key, and none expected. The person reading
    /// the refusal is in a browser, so it names the computer running Biorouter,
    /// not the page they are on.
    #[tokio::test]
    #[serial]
    async fn a_keyless_daemon_refuses_to_move_the_aws_endpoint_in_words_for_a_person() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        assert_eq!(
            user_action_proof(&http::HeaderMap::new()),
            UserActionProof::NoKeyInstalled
        );
        assert!(!biorouter_server::launch::expected_a_user_action_key());
        preconditions();

        let (status, sentence) =
            upsert_config(headers_with(None), write(ENDPOINT, ATTACKER, false))
                .await
                .expect_err("an unproven write of the Bedrock endpoint is refused");
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(sentence.contains("`biorouter configure`"), "{sentence}");
        assert!(
            sentence.contains("computer running Biorouter"),
            "{sentence}"
        );
        assert!(!sentence.contains("no proof"), "{sentence}");
        assert!(stored(ENDPOINT).is_none(), "nothing was written");

        if crate::test_sandbox::global_config_reads_secrets_from_a_file() {
            let (status, _) = upsert_config(headers_with(None), write(ENDPOINT, ATTACKER, true))
                .await
                .expect_err("the secret store is no way round it");
            assert_eq!(status, StatusCode::CONFLICT);
        }
        let (status, _) = upsert_config(
            headers_with(None),
            write("AWS_ENDPOINT_URL", ATTACKER, false),
        )
        .await
        .expect_err("nor is the SDK's generic endpoint");
        assert_eq!(status, StatusCode::CONFLICT);

        // The operator's own endpoint, set on the computer itself: re-saving it
        // moves nothing, and removing it would hand Bedrock back to AWS's own
        // host, which is a move.
        Config::global().set_param(ENDPOINT, VPC).unwrap();
        let _ = upsert_config(headers_with(None), write(ENDPOINT, VPC, false))
            .await
            .expect("re-saving the stored endpoint moves nothing");
        let (status, _) = remove_config(headers_with(None), removal(ENDPOINT))
            .await
            .expect_err("removing it is a move");
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(stored(ENDPOINT), Some(Value::from(VPC)));

        // A custom provider's URL, at its own route.
        let dir = biorouter::config::declarative_providers::custom_providers_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let saved = serde_json::json!({
            "name": "custom_lab",
            "engine": "openai",
            "display_name": "Lab gateway",
            "api_key_env": "CUSTOM_LAB_API_KEY",
            "base_url": "https://lab.example/v1",
            "models": [],
        });
        let file = dir.join("custom_lab.json");
        std::fs::write(&file, saved.to_string()).unwrap();
        let (status, sentence) = update_custom_provider(
            Path("custom_lab".to_string()),
            headers_with(None),
            Json(UpdateCustomProviderRequest {
                engine: "openai_compatible".to_string(),
                display_name: "Lab gateway".to_string(),
                api_url: ATTACKER.to_string(),
                api_key: String::new(),
                models: Vec::new(),
                supports_streaming: None,
                headers: None,
            }),
        )
        .await
        .expect_err("moving a custom provider's saved key is refused");
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(sentence.contains("type the key again"), "{sentence}");
        assert!(!sentence.contains("no proof"), "{sentence}");
        let kept: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(kept["base_url"], "https://lab.example/v1");
    }

    /// T3-SH-3: a declarative or custom provider's key was saved with no check,
    /// so a key the provider refuses replaced the working one.
    #[tokio::test]
    #[serial]
    async fn a_key_the_provider_refuses_is_not_saved_over_the_working_one() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        // The saved key is read and written only where secrets are kept in the
        // sandbox's file, never in the real keychain. The refusals themselves
        // touch no secret store, so they are asserted either way.
        let file_secrets = crate::test_sandbox::global_config_reads_secrets_from_a_file();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer working-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data": [{"id": "m"}]})),
            )
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_json(serde_json::json!({"error": {"message": "Invalid API Key"}})),
            )
            .with_priority(2)
            .mount(&server)
            .await;

        let dir = biorouter::config::declarative_providers::custom_providers_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let url = format!("{}/v1/chat/completions", server.uri());
        let saved = serde_json::json!({
            "name": "custom_lab",
            "engine": "openai",
            "display_name": "Lab gateway",
            "api_key_env": "CUSTOM_LAB_API_KEY",
            "base_url": url,
            "models": [{"name": "m", "context_limit": 128000}],
        });
        std::fs::write(dir.join("custom_lab.json"), saved.to_string()).unwrap();
        if file_secrets {
            Config::global()
                .set_secret("CUSTOM_LAB_API_KEY", &"working-key".to_string())
                .unwrap();
        }
        let update = |api_key: &str| {
            Json(UpdateCustomProviderRequest {
                engine: "openai_compatible".to_string(),
                display_name: "Lab gateway".to_string(),
                api_url: url.clone(),
                api_key: api_key.to_string(),
                models: vec!["m".to_string()],
                supports_streaming: None,
                headers: None,
            })
        };

        let (status, sentence) = update_custom_provider(
            Path("custom_lab".to_string()),
            headers_with(None),
            update("wrong-key"),
        )
        .await
        .expect_err("a key the provider refuses is not saved");
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            sentence.starts_with("Lab gateway rejected this key, so it was not saved:"),
            "{sentence}"
        );
        assert!(sentence.contains("Invalid API Key"), "{sentence}");
        if file_secrets {
            let kept: String = Config::global().get_secret("CUSTOM_LAB_API_KEY").unwrap();
            assert_eq!(kept, "working-key", "the working key was replaced");

            let _ = update_custom_provider(
                Path("custom_lab".to_string()),
                headers_with(None),
                update("working-key"),
            )
            .await
            .expect("a key the provider accepts is saved");
        }

        // A new provider is checked the same way before it is created.
        let (status, _) = create_custom_provider(Json(UpdateCustomProviderRequest {
            engine: "openai_compatible".to_string(),
            display_name: "Second lab".to_string(),
            api_url: url.clone(),
            api_key: "wrong-key".to_string(),
            models: vec!["m".to_string()],
            supports_streaming: None,
            headers: None,
        }))
        .await
        .expect_err("a new provider with a refused key is not created");
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!dir.join("custom_second_lab.json").exists());
    }

    /// The desktop's daemon: it holds a key, the app sends it, and a caller
    /// without it is a script or a model holding the daemon secret.
    #[tokio::test]
    #[serial]
    async fn a_keyed_daemon_refuses_an_unproven_aws_endpoint_and_takes_a_proven_one() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        install_test_user_action_key();
        preconditions();

        let (status, sentence) =
            upsert_config(headers_with(None), write(ENDPOINT, ATTACKER, false))
                .await
                .expect_err("an unproven write of the Bedrock endpoint is refused");
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            sentence.contains("no proof it came from them"),
            "{sentence}"
        );
        assert!(stored(ENDPOINT).is_none(), "nothing was written");

        let _ = upsert_config(
            headers_with(Some(TEST_USER_ACTION_KEY)),
            write(ENDPOINT, VPC, false),
        )
        .await
        .expect("the person's own change lands");
        assert_eq!(stored(ENDPOINT), Some(Value::from(VPC)));

        let (status, _) = remove_config(headers_with(None), removal(ENDPOINT))
            .await
            .expect_err("an unproven removal is a move");
        assert_eq!(status, StatusCode::CONFLICT);
        let _ = remove_config(headers_with(Some(TEST_USER_ACTION_KEY)), removal(ENDPOINT))
            .await
            .expect("the person's own removal lands");
        assert!(stored(ENDPOINT).is_none());

        // T3-SH-7: even the person's own write of a provider name has to name
        // one this daemon can build, or no new chat could start.
        let (status, sentence) = upsert_config(
            headers_with(Some(TEST_USER_ACTION_KEY)),
            write("BIOROUTER_PROVIDER", "bogus_provider_qa", false),
        )
        .await
        .expect_err("a provider nobody registered is refused");
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            sentence.contains("'bogus_provider_qa' is not a provider"),
            "{sentence}"
        );
        assert!(
            stored("BIOROUTER_PROVIDER").is_none(),
            "nothing was written"
        );
        let _ = upsert_config(
            headers_with(Some(TEST_USER_ACTION_KEY)),
            write("BIOROUTER_PROVIDER", "openai", false),
        )
        .await
        .expect("a registered provider is saved");
        assert_eq!(stored("BIOROUTER_PROVIDER"), Some(Value::from("openai")));

        // A custom provider added by another process since the daemon started
        // (its file is on disk, its registry entry is not) is a real provider.
        let dir = biorouter::config::declarative_providers::custom_providers_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("custom_elsewhere.json"),
            serde_json::json!({
                "name": "custom_elsewhere",
                "engine": "openai",
                "display_name": "Added elsewhere",
                "api_key_env": "CUSTOM_ELSEWHERE_API_KEY",
                "base_url": "https://elsewhere.example/v1",
                "models": [],
            })
            .to_string(),
        )
        .unwrap();
        let _ = upsert_config(
            headers_with(Some(TEST_USER_ACTION_KEY)),
            write("BIOROUTER_PROVIDER", "custom_elsewhere", false),
        )
        .await
        .expect("a provider on disk is known once the registry is re-read");
        assert_eq!(
            stored("BIOROUTER_PROVIDER"),
            Some(Value::from("custom_elsewhere"))
        );
    }
}
