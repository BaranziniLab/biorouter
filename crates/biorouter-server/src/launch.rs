//! What this daemon was **launched** with — the operator's own declaration,
//! sampled once before any route is mounted and never re-read.
//!
//! SD-12 (`docs/deployment/serve-decisions.md`) lets a daemon that holds no
//! user-action key bind a **private** provider to a brand-new chat with no proof
//! of a person, because nobody on such a daemon can produce one. The
//! justification is that the provider being bound is the *operator's* choice,
//! made at a terminal with `biorouter configure`.
//!
//! ⚠ **That sentence is only true of a value the operator wrote, and the file it
//! lives in is agent-writable.** DR-14's general filesystem deny is DEFERRED
//! (`docs/security/privacy-tiers.md`, *"Did not ship"*), the agent holds
//! `developer__shell`, and `Config`'s value cache is keyed on a `FileStamp` it
//! re-`stat`s on every read — so `config.yaml` is reloaded live and
//! `configured_new_session_provider()` reads whatever the file says **at request
//! time**. Without this module, a model on a keyless daemon whose operator
//! configured a *public* default could write a private provider into that file
//! and then `POST /agent/start` to mint a Private-capability chat with an
//! extension set of its own choosing.
//!
//! So the exemption is **pinned** to this snapshot. The bind still reads the
//! configuration — an operator who edits the file and restarts the daemon is
//! served — but SD-12's keyless exemption applies only while the
//! capability-deciding configuration still matches what the daemon started with.
//! That is SD-1's own sentence made true of the door: *"the tier implied by the
//! operator's `biorouter configure` choice then holds for every session in that
//! daemon."*
//!
//! It also records whether the **launcher declared** it would hand over a
//! user-action key, which is what separates two states `UserActionProof`
//! deliberately collapses into one: a deployment where no proof can ever exist,
//! and a desktop daemon whose key did not arrive.
//!
//! ⚠ **This is not DR-14, and must not be read as a substitute for it.** A shell
//! is still a shell: a model that can write `config.yaml` can also read the
//! session store and the knowledge bases directly, and can start a *second*
//! `biorouterd` of its own. What this module closes is one door's *stated*
//! guarantee, so that the record does not rest on a claim the tree contradicts.

use std::sync::{PoisonError, RwLock};

/// A launcher that hands this daemon a user-action digest on stdin declares
/// itself here.
///
/// It exists so the daemon can tell *"no key was ever meant to arrive"*
/// (`biorouter serve`, which spawns with `Stdio::null()`, or a hand-run
/// `biorouterd agent`) from *"one was, and did not"* — a fault to repair rather
/// than a deployment shape. Set unconditionally on the spawn path in
/// `ui/desktop/src/biorouterd.ts`, **including** when that path finds no key to
/// send, because that case is precisely the one worth naming.
///
/// In the environment rather than on stdin on purpose: it is not a credential and
/// nothing is authenticated by it. A value that can only make this daemon
/// *stricter* is safe to read from a place the model can see but not write.
pub const USER_ACTION_EXPECTED_ENV: &str = "BIOROUTER_USER_ACTION_EXPECTED";

/// Reported by [`capability_config_moved_since_launch`] when this process never
/// recorded a launch state at all. Not a config key — a sentinel, so the caller
/// fails closed instead of reading "nothing moved".
pub const NO_LAUNCH_STATE_RECORDED: &str = "<no launch state recorded>";

#[derive(Debug)]
struct LaunchState {
    /// `(key, value at launch)` for every key in [`pinned_config_keys`].
    capability_config: Vec<(&'static str, Option<String>)>,
    /// Did whoever started this daemon say it would send a user-action key?
    launcher_declared_a_user_action_key: bool,
}

static LAUNCH: RwLock<Option<LaunchState>> = RwLock::new(None);

/// The configuration keys whose value at request time must still match the value
/// this daemon started with, for SD-12's keyless exemption to apply.
///
/// [`biorouter::privacy::CAPABILITY_CONFIG_KEYS`] **verbatim** — the same list
/// `/config/upsert` and `/config/remove` already use to decide *"is this write a
/// tier raise?"* — plus `BIOROUTER_MODEL`.
///
/// Reusing that list rather than writing a second one is the whole point:
/// `privacy::config_keys`'s scan of the tier-input files is what keeps it honest,
/// so a key that starts deciding capability is pinned here without anyone
/// remembering to, and one that stops deciding it leaves. A hand-written second
/// list would be a third answer to a question that already has two agreeing ones.
///
/// ⚠ **The provider name alone is not enough.** Flipping `OLLAMA_HOST` to
/// loopback moves `ollama` from Public to Private with `BIOROUTER_PROVIDER`
/// untouched (`self_hosted_tier`) — the same escalation through another key.
///
/// `BIOROUTER_MODEL` is **not** a capability key — no `tier()` implementation
/// reads the model name — and it is pinned here for a different reason: the
/// exemption is for the operator's own declaration, and `/agent/start` binds
/// *both* halves of it (`configured_new_session_provider`). Its classification
/// lives in `privacy::config_keys::NOT_CAPABILITY_CONFIG_KEYS`.
pub fn pinned_config_keys() -> impl Iterator<Item = &'static str> {
    biorouter::privacy::CAPABILITY_CONFIG_KEYS
        .iter()
        .copied()
        .chain(std::iter::once("BIOROUTER_MODEL"))
}

fn sample_capability_config() -> Vec<(&'static str, Option<String>)> {
    let config = biorouter::config::Config::global();
    pinned_config_keys()
        .map(|key| (key, config.get_param::<String>(key).ok()))
        .collect()
}

/// Record what this daemon was launched with.
///
/// Called ONCE from `commands::agent::run`, after the stdin digest read and
/// before `AppState::new()` — so no request can be served against an unrecorded
/// launch state, and so the sample is taken before anything in this process could
/// have written `config.yaml` itself.
///
/// It overwrites rather than being write-once, because the integration tests that
/// exercise a keyless daemon have to stand up more than one launch posture in a
/// single binary (the config overrides are a `tokio` task-local scoped to a
/// future, so the sample must be taken inside one). Production has exactly one
/// call site.
pub fn record_launch_state(launcher_declared_a_user_action_key: bool) {
    let recorded = LaunchState {
        capability_config: sample_capability_config(),
        launcher_declared_a_user_action_key,
    };
    *LAUNCH.write().unwrap_or_else(PoisonError::into_inner) = Some(recorded);
}

/// The first pinned key whose value no longer matches what this daemon started
/// with, or `None` while the operator's declaration is unchanged.
///
/// ⚠ **Fails closed.** A process that never recorded a launch state reports
/// [`NO_LAUNCH_STATE_RECORDED`] rather than `None`: the one caller uses this to
/// decide whether to *skip* a privacy proof, and a missing snapshot must not read
/// as a clean one.
pub fn capability_config_moved_since_launch() -> Option<String> {
    let guard = LAUNCH.read().unwrap_or_else(PoisonError::into_inner);
    let Some(state) = guard.as_ref() else {
        return Some(NO_LAUNCH_STATE_RECORDED.to_string());
    };
    let config = biorouter::config::Config::global();
    state
        .capability_config
        .iter()
        .find(|(key, at_launch)| config.get_param::<String>(key).ok() != *at_launch)
        .map(|(key, _)| (*key).to_string())
}

/// Did whoever started this daemon declare it would hand over a user-action key?
///
/// Read from the recorded launch state, never from the environment at request
/// time, for the reason the whole module exists: the answer is a property of the
/// launch, not of the moment.
///
/// `false` when nothing was recorded, which is not a fail-open reading — the
/// composite gate is closed by [`capability_config_moved_since_launch`], which
/// reports drift in exactly that situation.
pub fn expected_a_user_action_key() -> bool {
    LAUNCH
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .is_some_and(|state| state.launcher_declared_a_user_action_key)
}

/// Did the launcher set [`USER_ACTION_EXPECTED_ENV`]?
///
/// Read exactly once, by `commands::agent::run`, and then frozen into the launch
/// state. Anything other than unset, empty, `0` or `false` counts as a
/// declaration — a launcher that says anything at all here is claiming it sends a
/// key, and the stricter reading is the safe one.
pub fn launcher_declared_a_user_action_key_in_env() -> bool {
    match std::env::var(USER_ACTION_EXPECTED_ENV) {
        Ok(value) => {
            let value = value.trim();
            !(value.is_empty() || value == "0" || value.eq_ignore_ascii_case("false"))
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pinned set is the capability list plus the model, and it is *derived*
    /// rather than transcribed — so this asserts the derivation, not a copy.
    #[test]
    fn the_pinned_set_is_the_capability_list_plus_the_model() {
        let pinned: Vec<&str> = pinned_config_keys().collect();
        for key in biorouter::privacy::CAPABILITY_CONFIG_KEYS {
            assert!(
                pinned.contains(key),
                "{key} decides capability but is not pinned to the launch configuration"
            );
        }
        assert!(pinned.contains(&"BIOROUTER_MODEL"));
        assert_eq!(
            pinned.len(),
            biorouter::privacy::CAPABILITY_CONFIG_KEYS.len() + 1,
            "the pinned set grew a key of its own; it must stay a derivation: {pinned:?}"
        );
    }

    /// The reading that matters most: an unrecorded launch state is drift, never
    /// agreement.
    ///
    /// Stated against the function rather than against the static, because this
    /// binary's tests share one process and another may have recorded a state
    /// already.
    #[test]
    fn an_unrecorded_launch_state_reads_as_drift() {
        let unrecorded = LAUNCH
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none();
        if unrecorded {
            assert_eq!(
                capability_config_moved_since_launch().as_deref(),
                Some(NO_LAUNCH_STATE_RECORDED)
            );
        }
        assert!(
            !expected_a_user_action_key() || !unrecorded,
            "an unrecorded launch state must not claim a key was expected"
        );
    }

    #[test]
    fn only_a_launcher_that_says_nothing_is_read_as_sending_no_key() {
        for (value, declared) in [
            (Some("1"), true),
            (Some("true"), true),
            (Some("yes"), true),
            (Some(""), false),
            (Some("0"), false),
            (Some("false"), false),
            (Some("FALSE"), false),
            (None, false),
        ] {
            let _guard = env_lock::lock_env([(USER_ACTION_EXPECTED_ENV, value)]);
            assert_eq!(
                launcher_declared_a_user_action_key_in_env(),
                declared,
                "{value:?} was read the wrong way"
            );
        }
    }

    /// A recorded launch state agrees with the configuration it was sampled
    /// from, and disagrees the moment one of the pinned keys moves.
    #[tokio::test]
    async fn a_pinned_key_that_moves_after_launch_is_named() {
        use biorouter::config::with_config_overrides;
        use std::collections::HashMap;

        let launched_with = HashMap::from([
            ("BIOROUTER_PROVIDER".to_string(), "ollama".to_string()),
            ("BIOROUTER_MODEL".to_string(), "stub-model".to_string()),
            (
                "OLLAMA_HOST".to_string(),
                "https://ollama.example".to_string(),
            ),
        ]);
        with_config_overrides(launched_with.clone(), async {
            record_launch_state(false);
            assert_eq!(capability_config_moved_since_launch(), None);
        })
        .await;

        // The provider name is untouched; only the endpoint moved — which is the
        // case a name-only pin would have waved through.
        let mut flipped = launched_with.clone();
        flipped.insert("OLLAMA_HOST".to_string(), "http://127.0.0.1:1".to_string());
        with_config_overrides(flipped, async {
            assert_eq!(
                capability_config_moved_since_launch().as_deref(),
                Some("OLLAMA_HOST")
            );
        })
        .await;

        let mut swapped = launched_with;
        swapped.insert("BIOROUTER_PROVIDER".to_string(), "versa_azure".to_string());
        with_config_overrides(swapped, async {
            assert_eq!(
                capability_config_moved_since_launch().as_deref(),
                Some("BIOROUTER_PROVIDER")
            );
        })
        .await;
    }
}
