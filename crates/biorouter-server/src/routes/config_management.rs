use crate::routes::utils::check_provider_configured;
use crate::state::AppState;
use axum::routing::put;
use axum::{
    extract::Path,
    routing::{delete, get, post},
    Json, Router,
};
use biorouter::config::declarative_providers::LoadedProvider;
use biorouter::config::paths::Paths;
use biorouter::config::ExtensionEntry;
use biorouter::config::{Config, ConfigError, ConfigWriteFailure};
use biorouter::model::ModelConfig;
use biorouter::privacy::ProviderTier;
use biorouter::providers::auto_detect::{detect_provider_from_api_key, detectable_providers};
use biorouter::providers::base::{ProviderAffiliation, ProviderMetadata, ProviderType};
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
}

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
                                      'standard' or 'strict'"),
        (status = 403, description = "Refused: `BIOROUTER_PRIVACY_TIERS` is the master privacy \
                                      switch and may only be written from Settings > Privacy, \
                                      with its typed confirmation, or (issue #56, DR-27) \
                                      relaxing `BIOROUTER_PRIVACY_MIXING_POLICY` needed a system \
                                      authentication that did not happen"),
        (status = 409, description = "Refused by a privacy boundary (issue #56, DR-16): the key \
                                      decides what privacy capability new chats start at, so \
                                      writing it requires proof the request came from the user. \
                                      Also (DR-27) `BIOROUTER_PRIVACY_MIXING_POLICY`, which is \
                                      user-only in every mode"),
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

    let result = config.set(&query.key, &query.value, query.is_secret);

    match result {
        Ok(_) => Ok(Json(Value::String(format!("Upserted key {}", query.key)))),
        Err(_) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to upsert key {}", query.key),
        )),
    }
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
         configuration value: change it in Settings > Privacy, which asks the user to type the \
         confirmation phrase and explains what turning it off exposes."
    )
}

#[utoipa::path(
    post,
    path = "/config/remove",
    request_body = ConfigKeyQuery,
    responses(
        (status = 200, description = "Configuration value removed successfully", body = String),
        (status = 403, description = "Refused: `BIOROUTER_PRIVACY_TIERS` is the master privacy \
                                      switch and may only be changed from Settings > Privacy, \
                                      never removed, and (issue #56, DR-27) \
                                      `BIOROUTER_PRIVACY_MIXING_POLICY` is set, never deleted"),
        (status = 404, description = "Configuration key not found"),
        (status = 409, description = "Refused by a privacy boundary (issue #56, DR-16): the key \
                                      decides what privacy capability new chats start at, and a \
                                      delete restores its default, so it requires proof the \
                                      request came from the user"),
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
                 Settings > Privacy, which is the one door that proves a human and asks the \
                 operating system before it relaxes anything.",
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

const SECRET_MASK_SHOW_LEN: usize = 8;

fn mask_secret(secret: Value) -> String {
    let as_string = match secret {
        Value::String(s) => s,
        _ => serde_json::to_string(&secret).unwrap_or_else(|_| secret.to_string()),
    };

    let chars: Vec<_> = as_string.chars().collect();
    let show_len = std::cmp::min(chars.len() / 2, SECRET_MASK_SHOW_LEN);
    let visible: String = chars.iter().take(show_len).collect();
    let mask = "*".repeat(chars.len() - show_len);

    format!("{}{}", visible, mask)
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
                    masked_value: mask_secret(value),
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
    Ok(Json(format!("Removed extension {}", name)))
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
    let providers_response: Vec<ProviderDetails> =
        futures::future::join_all(providers.into_iter().map(
            |(metadata, provider_type)| async move {
                let is_configured = check_provider_configured(&metadata, provider_type);
                // Issue #56, DR-26. Both resolved from the instance, never from
                // the name — see `resolve_provider_axes`.
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
                }
            },
        ))
        .await;

    Ok(Json(providers_response))
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
        Ok(None) => Ok(Json(Vec::new())),
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
) -> Result<Json<String>, StatusCode> {
    let config = biorouter::config::declarative_providers::create_custom_provider(
        &request.engine,
        request.display_name,
        request.api_url,
        request.api_key,
        request.models,
        request.supports_streaming,
        request.headers,
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
        (status = 500, description = "Internal server error")
    )
)]
pub async fn update_custom_provider(
    Path(id): Path<String>,
    Json(request): Json<UpdateCustomProviderRequest>,
) -> Result<Json<String>, StatusCode> {
    biorouter::config::declarative_providers::update_custom_provider(
        &id,
        &request.engine,
        request.display_name,
        request.api_url,
        request.api_key,
        request.models,
        request.supports_streaming,
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if let Err(e) = biorouter::providers::refresh_custom_providers().await {
        tracing::warn!("Failed to refresh custom providers after update: {}", e);
    }

    Ok(Json(format!("Updated custom provider: {}", id)))
}

#[utoipa::path(
    post,
    path = "/config/check_provider",
    request_body = CheckProviderRequest,
)]
pub async fn check_provider(
    Json(CheckProviderRequest { provider }): Json<CheckProviderRequest>,
) -> Result<(), (StatusCode, String)> {
    create_with_default_model(&provider)
        .await
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    Ok(())
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

    use http::HeaderMap;

    use super::*;

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
/// `AppState`, and building one opens the developer's REAL session database
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
        // Its own config root, or this test writes the acknowledgement into the
        // developer's real `~/.config/biorouter` and every later run of it
        // starts already-acknowledged.
        let dir = tempfile::TempDir::new().unwrap();
        let _env = env_lock::lock_env([(
            "BIOROUTER_PATH_ROOT",
            Some(dir.path().to_str().expect("utf-8 temp path")),
        )]);
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
        // The renderer holds no English of its own; this is the wire it gets it
        // over. Compared against the constants themselves, so a second copy
        // written into this handler fails here rather than in a screenshot.
        let dir = tempfile::TempDir::new().unwrap();
        let _env = env_lock::lock_env([(
            "BIOROUTER_PATH_ROOT",
            Some(dir.path().to_str().expect("utf-8 temp path")),
        )]);
        let served = get_privacy_disclosure().await.0;
        assert_eq!(served.long, biorouter::privacy::disclosure::COPY_LONG);
        assert_eq!(served.short, biorouter::privacy::disclosure::COPY_SHORT);
        assert_eq!(
            served.title_template,
            biorouter::privacy::disclosure::COPY_TITLE_TEMPLATE
        );
    }
}
