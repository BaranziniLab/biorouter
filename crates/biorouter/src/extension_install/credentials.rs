//! The secret-safe half of an extension install (#117).
//!
//! # The property this module exists to hold
//!
//! **A credential goes from the trusted surface to the credential store, and
//! nothing else ever sees it.** Not the model, not the provider, not a session
//! row, not a log line, not a process argument, not a diagnostics bundle.
//!
//! The obvious implementation — reuse MCP elicitation and give the form a
//! `type="password"` — fails this on its first turn, and fails it invisibly:
//! [`crate::conversation::message::ActionRequiredData::ElicitationResponse`]
//! carries `user_data`, the desktop marks that message `agentVisible: true`, and
//! `Agent::reply` forwards the whole object to the pending request. Masking the
//! characters would hide the secret from the person typing it and from nobody
//! else.
//!
//! So the values never enter the conversation transport at all:
//!
//! ```text
//!   transaction ──park(Secrets{keys, destination})──▶ card  (KEY NAMES ONLY)
//!                                                      │
//!        user types into a trusted surface ────────────┘
//!                                                      │
//!        POST /action_required/secrets  (X-User-Action)│
//!                                                      ▼
//!                                            submit_credentials
//!                                              ├── secret  → OS keyring
//!                                              ├── setting → this registry
//!                                              └── resolve(SecretsConfigured{names})
//!   transaction ◀────── configured_keys: ["SPOKEAGENT_PASSCODE"] ──────┘
//! ```
//!
//! [`crate::pending_user_action::PendingUserActions::resolve`] enforces the
//! last step rather than trusting it: a `Secrets` request answered with a
//! value-bearing `Provided` is `Rejected` and the caller stays parked.
//!
//! # Why a registry beside the park
//!
//! A card carries `{key, label, description, required}` and deliberately no
//! `secret` flag — it is the *ask*, and widening it would be one more place a
//! value could be attached. But the writer has to tell a credential (keyring,
//! name recorded in `env_keys`) from an ordinary setting (`envs` in
//! `config.yaml`), and that fact lives in the bundle's manifest. So the
//! transaction registers the manifest's declarations here, keyed by the park id,
//! and the answering route reads them. The route therefore needs to know nothing
//! about extensions, bundles or installs.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::config::Config;
use crate::conversation::message::{SecretDestination, SecretKeyRequest};
use crate::pending_user_action::{
    PendingUserActions, ResolveOutcome, SecretsRequest, UserActionOutcome, UserActionRequest,
};

use super::brxt::BrxtEnvVar;

/// How long a credential card waits for a person before it gives up.
///
/// Long, because a user answering one is routinely going to another window to
/// find an API key, and short enough that a forgotten dialog cannot hold an
/// install open indefinitely. A caller with a tighter bound (a bridged call
/// blocked on an HTTP response) passes its own.
pub const DEFAULT_CREDENTIAL_TTL: Duration = Duration::from_secs(15 * 60);

/// What the answering surface must do with the values it collects.
#[derive(Debug, Clone)]
pub struct CredentialSpec {
    /// Where the values go, as published on the card.
    pub destination: SecretDestination,
    /// The manifest's declaration for each key on the card. The `secret` flag
    /// here is what decides keyring-versus-config; see the module header.
    pub vars: Vec<BrxtEnvVar>,
}

impl CredentialSpec {
    fn var(&self, key: &str) -> Option<&BrxtEnvVar> {
        self.vars.iter().find(|v| v.key == key)
    }

    fn required_vars(&self) -> impl Iterator<Item = &BrxtEnvVar> {
        self.vars.iter().filter(|v| v.required)
    }
}

struct Entry {
    spec: CredentialSpec,
    /// Non-secret values the surface supplied, waiting for the transaction to
    /// fold them into the extension's config. Never a credential — a credential
    /// is in the OS store by the time this is written.
    settings: HashMap<String, String>,
}

/// The process-global map from a parked card to what answering it means.
#[derive(Default)]
pub struct CredentialRequests {
    entries: Mutex<HashMap<String, Entry>>,
}

/// What [`submit_credentials`] did.
#[derive(Debug, Clone, PartialEq)]
pub enum SubmitOutcome {
    /// Written and the parked install released.
    Configured { configured_keys: Vec<String> },
    /// Required keys are still empty. **The install stays parked** so the user
    /// can correct the form rather than starting over.
    Incomplete { missing: Vec<String> },
    /// Nothing is parked under that id: a double submit, a card answered after
    /// the turn ended, a stale window.
    Unknown,
    /// The values could not be stored. The install is released as `Failed` so
    /// it rolls back rather than registering an extension that cannot
    /// authenticate.
    Failed { reason: String },
}

impl CredentialRequests {
    pub fn global() -> &'static Arc<Self> {
        static INSTANCE: once_cell::sync::Lazy<Arc<CredentialRequests>> =
            once_cell::sync::Lazy::new(|| Arc::new(CredentialRequests::default()));
        &INSTANCE
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn register(&self, id: &str, spec: CredentialSpec) {
        self.lock().insert(
            id.to_string(),
            Entry {
                spec,
                settings: HashMap::new(),
            },
        );
    }

    /// The spec for a live card, for a surface that wants to know what it is
    /// answering before it answers.
    pub fn spec(&self, id: &str) -> Option<CredentialSpec> {
        self.lock().get(id).map(|e| e.spec.clone())
    }

    /// Take the non-secret settings written under `id`. Called once, by the
    /// transaction, after its park resolves.
    fn take_settings(&self, id: &str) -> HashMap<String, String> {
        self.lock()
            .get_mut(id)
            .map(|e| std::mem::take(&mut e.settings))
            .unwrap_or_default()
    }

    fn forget(&self, id: &str) {
        self.lock().remove(id);
    }
}

/// A published credential card, before anyone has answered it.
///
/// Separate from the wait so a caller — or a test — can learn the id the card
/// was published under while it is still open. That id is the only handle a
/// surface needs to answer it.
pub struct ParkedCredentials {
    parked: crate::pending_user_action::PendingUserAction,
}

impl ParkedCredentials {
    /// The id the answering surface posts back.
    pub fn id(&self) -> &str {
        self.parked.id()
    }

    /// Park until answered, `ttl` elapses, or `cancel` trips.
    ///
    /// Returns the outcome and any **non-secret** settings the surface
    /// collected. A credential is in the OS store by the time this returns and
    /// is not among them.
    pub async fn wait(
        self,
        ttl: Duration,
        cancel: Option<&CancellationToken>,
    ) -> (UserActionOutcome, HashMap<String, String>) {
        let id = self.parked.id().to_string();
        let outcome = self.parked.wait(ttl, cancel).await;
        let settings = CredentialRequests::global().take_settings(&id);
        CredentialRequests::global().forget(&id);
        (outcome, settings)
    }
}

/// Publish a credential card to `session_id`.
///
/// `owner` groups parks that die together — an install started by a bridged tool
/// call passes the bridge's nonce so the card dies with the turn.
pub fn park_credentials(
    session_id: Option<&str>,
    owner: Option<&str>,
    prompt: String,
    spec: CredentialSpec,
) -> ParkedCredentials {
    let keys: Vec<SecretKeyRequest> = spec.vars.iter().map(BrxtEnvVar::as_key_request).collect();
    let request = UserActionRequest::Secrets(SecretsRequest {
        prompt,
        keys,
        destination: spec.destination.clone(),
    });

    let parked = PendingUserActions::global().park(session_id, owner, request);
    // Registered BEFORE anyone can answer, for the same reason `park` registers
    // before it publishes: a surface fast enough to answer the card immediately
    // must find a spec to write against.
    CredentialRequests::global().register(parked.id(), spec);
    ParkedCredentials { parked }
}

/// Park a credential card and wait for a person. The transaction's path.
pub async fn request_credentials(
    session_id: Option<&str>,
    owner: Option<&str>,
    prompt: String,
    spec: CredentialSpec,
    ttl: Duration,
    cancel: Option<&CancellationToken>,
) -> (UserActionOutcome, HashMap<String, String>) {
    park_credentials(session_id, owner, prompt, spec)
        .wait(ttl, cancel)
        .await
}

/// Answer a parked credential card: store the values, release the install.
///
/// **This is the only way values are written**, and every surface goes through
/// it — the desktop route, the CLI's no-echo prompt, and any future one. The
/// values are consumed here and never returned, logged, or attached to the
/// outcome.
///
/// The `values` map is taken by value and dropped before this returns.
pub fn submit_credentials(id: &str, values: HashMap<String, String>) -> SubmitOutcome {
    let Some(spec) = CredentialRequests::global().spec(id) else {
        // Either nothing was ever parked, or it has already been answered.
        return SubmitOutcome::Unknown;
    };
    if !PendingUserActions::global().is_pending(id) {
        CredentialRequests::global().forget(id);
        return SubmitOutcome::Unknown;
    }

    let missing: Vec<String> = spec
        .required_vars()
        .filter(|var| {
            values
                .get(&var.key)
                .map(|v| v.trim().is_empty())
                .unwrap_or(true)
                && !var.has_stored_secret()
        })
        .map(|var| var.key.clone())
        .collect();
    if !missing.is_empty() {
        // Deliberately does NOT resolve: the user is looking at the form and can
        // fix it. Releasing the install here would make a typo into a rollback.
        return SubmitOutcome::Incomplete { missing };
    }

    let config = Config::global();
    let mut configured_keys: Vec<String> = Vec::new();
    let mut settings: HashMap<String, String> = HashMap::new();
    let mut written_secrets: Vec<String> = Vec::new();

    for (key, value) in values.into_iter() {
        if value.trim().is_empty() {
            continue;
        }
        // A key the card never asked for is dropped rather than written. The
        // card is generated from a validated manifest; anything else arriving
        // here came from a client that made it up, and writing it would let a
        // form post decide what lands in the credential store.
        let Some(var) = spec.var(&key) else {
            debug!("Dropping `{key}`: the credential card did not ask for it");
            continue;
        };
        if var.secret {
            if let Err(e) = config.set_secret(&key, &value) {
                // Roll the partial write back before reporting: half a
                // credential set is worse than none, because the next attempt
                // would see the stored half as "already configured".
                for written in &written_secrets {
                    let _ = config.delete_secret(written);
                }
                let reason = format!("Could not store `{key}` in the credential store: {e}");
                CredentialRequests::global().forget(id);
                PendingUserActions::global().resolve_trusted_sessionless_secret(
                    id,
                    UserActionOutcome::Failed {
                        reason: reason.clone(),
                    },
                );
                return SubmitOutcome::Failed { reason };
            }
            written_secrets.push(key.clone());
        } else {
            settings.insert(key.clone(), value);
        }
        configured_keys.push(key);
    }

    configured_keys.sort();
    {
        let requests = CredentialRequests::global();
        let mut entries = requests.lock();
        if let Some(entry) = entries.get_mut(id) {
            entry.settings = settings;
        }
    }

    match PendingUserActions::global().resolve_trusted_sessionless_secret(
        id,
        UserActionOutcome::SecretsConfigured {
            configured_keys: configured_keys.clone(),
        },
    ) {
        ResolveOutcome::Delivered => SubmitOutcome::Configured { configured_keys },
        // `Rejected` here would mean this id is not a secrets request at all —
        // a routing bug, not a user error. `Unknown` means the waiter went away
        // between the check above and now.
        other => {
            debug!("Credential submission for {id} was not delivered: {other:?}");
            CredentialRequests::global().forget(id);
            SubmitOutcome::Unknown
        }
    }
}

/// Dismiss a parked credential card without values.
pub fn cancel_credentials(id: &str) -> bool {
    CredentialRequests::global().forget(id);
    PendingUserActions::global()
        .resolve_trusted_sessionless_secret(id, UserActionOutcome::Cancelled)
        == ResolveOutcome::Delivered
}

/// Remove credentials this install wrote. Used by rollback, so a failed or
/// cancelled install does not leave a passcode in the machine's keychain for an
/// extension that is not there.
pub fn revoke(keys: &[String]) {
    let config = Config::global();
    for key in keys {
        let _ = config.delete_secret(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(key: &str, required: bool, secret: bool) -> BrxtEnvVar {
        BrxtEnvVar {
            key: key.to_string(),
            required,
            auto_propagate: false,
            default: None,
            description: String::new(),
            secret,
        }
    }

    fn spec(vars: Vec<BrxtEnvVar>) -> CredentialSpec {
        CredentialSpec {
            destination: SecretDestination::ExtensionEnv {
                extension_name: "spokeagent".to_string(),
            },
            vars,
        }
    }

    /// The card is the ask, and an ask is not an answer. Serialising the
    /// published request must never produce a value — this is the assertion that
    /// fails if somebody widens `SecretKeyRequest` to carry one.
    #[test]
    fn a_published_card_carries_key_names_and_nothing_else() {
        let vars = [var("SPOKEAGENT_PASSCODE", true, true)];
        let keys: Vec<SecretKeyRequest> = vars.iter().map(BrxtEnvVar::as_key_request).collect();
        let json = serde_json::to_string(&keys).unwrap();
        assert!(json.contains("SPOKEAGENT_PASSCODE"));
        assert!(!json.to_lowercase().contains("value"));
    }

    #[test]
    fn submitting_against_an_unknown_id_changes_nothing() {
        assert_eq!(
            submit_credentials("no-such-card", HashMap::new()),
            SubmitOutcome::Unknown
        );
    }

    /// An empty required field leaves the install parked. The alternative —
    /// resolving as failed — turns a typo into a rollback the user then has to
    /// start over from.
    #[tokio::test]
    async fn a_missing_required_value_leaves_the_install_parked() {
        let key = format!("MISSING_TOKEN_{}", uuid::Uuid::new_v4().simple()).to_uppercase();
        let registry = PendingUserActions::global();
        let parked = registry.park(
            Some("s-incomplete"),
            None,
            UserActionRequest::Secrets(SecretsRequest {
                prompt: "Configure".to_string(),
                keys: vec![var(&key, true, true).as_key_request()],
                destination: SecretDestination::Keyring,
            }),
        );
        let id = parked.id().to_string();
        CredentialRequests::global().register(&id, spec(vec![var(&key, true, true)]));

        let outcome = crate::config::with_config_overrides(
            HashMap::from([(key.clone(), "  ".to_string())]),
            async { submit_credentials(&id, HashMap::from([(key.clone(), "  ".into())])) },
        )
        .await;
        assert_eq!(outcome, SubmitOutcome::Incomplete { missing: vec![key] });
        assert!(registry.is_pending(&id), "the install must still be parked");

        CredentialRequests::global().forget(&id);
        registry.resolve_trusted_sessionless_secret(&id, UserActionOutcome::Cancelled);
        drop(parked);
    }

    #[tokio::test]
    async fn a_stored_same_named_secret_does_not_satisfy_an_ordinary_setting() {
        let registry = PendingUserActions::global();
        let parked = registry.park(
            Some("s-ordinary-setting"),
            None,
            UserActionRequest::Secrets(SecretsRequest {
                prompt: "Configure".to_string(),
                keys: vec![var("SHARED_NAME", true, false).as_key_request()],
                destination: SecretDestination::Keyring,
            }),
        );
        let id = parked.id().to_string();
        CredentialRequests::global().register(&id, spec(vec![var("SHARED_NAME", true, false)]));

        let outcome = crate::config::with_config_overrides(
            HashMap::from([(String::from("SHARED_NAME"), String::from("stored-secret"))]),
            async {
                submit_credentials(
                    &id,
                    HashMap::from([(String::from("SHARED_NAME"), String::new())]),
                )
            },
        )
        .await;
        assert_eq!(
            outcome,
            SubmitOutcome::Incomplete {
                missing: vec!["SHARED_NAME".to_string()]
            }
        );
        assert!(
            registry.is_pending(&id),
            "the ordinary setting must remain pending"
        );

        CredentialRequests::global().forget(&id);
        registry.resolve_trusted_sessionless_secret(&id, UserActionOutcome::Cancelled);
        drop(parked);
    }

    /// BR-62's property, extended to credentials: two installs in flight cannot
    /// answer each other. The registry keys every park by its own id and drops a
    /// decision for an id nobody holds rather than re-aiming it.
    #[tokio::test]
    async fn one_card_cannot_satisfy_another_installs_request() {
        let registry = PendingUserActions::global();
        let first = registry.park(
            Some("s-a"),
            None,
            UserActionRequest::Secrets(SecretsRequest {
                prompt: "A".to_string(),
                keys: vec![var("A_KEY", false, false).as_key_request()],
                destination: SecretDestination::Keyring,
            }),
        );
        let second = registry.park(
            Some("s-b"),
            None,
            UserActionRequest::Secrets(SecretsRequest {
                prompt: "B".to_string(),
                keys: vec![var("B_KEY", false, false).as_key_request()],
                destination: SecretDestination::Keyring,
            }),
        );
        let (id_a, id_b) = (first.id().to_string(), second.id().to_string());
        CredentialRequests::global().register(&id_a, spec(vec![var("A_KEY", false, false)]));
        CredentialRequests::global().register(&id_b, spec(vec![var("B_KEY", false, false)]));

        // Answering A releases A and leaves B exactly where it was.
        let outcome = submit_credentials(&id_a, HashMap::from([("A_KEY".into(), "a".into())]));
        assert!(matches!(outcome, SubmitOutcome::Configured { .. }));
        assert!(!registry.is_pending(&id_a));
        assert!(registry.is_pending(&id_b), "B must not have been released");

        // And a second answer to A now lands nowhere at all.
        assert_eq!(
            submit_credentials(&id_a, HashMap::from([("A_KEY".into(), "again".into())])),
            SubmitOutcome::Unknown
        );

        CredentialRequests::global().forget(&id_b);
        registry.resolve_trusted_sessionless_secret(&id_b, UserActionOutcome::Cancelled);
        drop(first);
        drop(second);
    }

    /// A client that posts a key the card never asked for does not get to choose
    /// what enters the credential store.
    #[tokio::test]
    async fn a_key_the_card_did_not_ask_for_is_dropped() {
        let registry = PendingUserActions::global();
        let parked = registry.park(
            Some("s-extra"),
            None,
            UserActionRequest::Secrets(SecretsRequest {
                prompt: "Configure".to_string(),
                keys: vec![var("WANTED", false, false).as_key_request()],
                destination: SecretDestination::Keyring,
            }),
        );
        let id = parked.id().to_string();
        CredentialRequests::global().register(&id, spec(vec![var("WANTED", false, false)]));

        let outcome = submit_credentials(
            &id,
            HashMap::from([
                ("WANTED".to_string(), "yes".to_string()),
                ("ANTHROPIC_API_KEY".to_string(), "sk-stolen".to_string()),
            ]),
        );
        assert_eq!(
            outcome,
            SubmitOutcome::Configured {
                configured_keys: vec!["WANTED".to_string()]
            }
        );
        drop(parked);
    }
}
