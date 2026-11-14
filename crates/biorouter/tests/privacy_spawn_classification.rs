//! Issue #56, finding 11: **turning the master switch off must not mint rows
//! that turning it back on cannot correct.**
//!
//! DR-15's switch is allowed to suspend *enforcement*. It is not allowed to
//! corrupt *classification*, and the two are different things: the gates in
//! `apply_settings_overrides` are behind `privacy_tiers_enabled()`, so with the
//! switch off a Private-classified chat may be bound to and may run a public
//! provider — and its subagent's row, if it were stamped from the child's
//! capability alone, would be born permanently `public`. Permanently, because
//! re-enabling never revisits a row (AR-7). The documented opt-out would then be
//! a way to launder a private chat's lineage into public rows, which is a worse
//! outcome than the enforcement it was asked to suspend.
//!
//! ⚠ **Why this is its own integration binary and not a `--lib` test.**
//! `set_privacy_tiers_enabled` mutates a PROCESS-GLOBAL atomic and `cargo test`
//! runs a crate's unit tests in parallel threads of one process, so flipping it
//! from a lib test would make `crates/biorouter/src/**`'s privacy tests read
//! `false` while asserting — spuriously failing, or worse, passing while
//! asserting nothing. Each `tests/*.rs` file is its own process. This is the
//! same reasoning, and the same guard shape, as `privacy_toggle.rs`.
//!
//! ⚠ **Why it is not a row in `privacy_toggle.rs`'s matrix.** That matrix asserts
//! what each gate does in each column. This is the opposite kind of claim — an
//! INVARIANT that must hold identically in both columns, like its row 17 (the
//! copy path) — and its subject is a SEQUENCE (classify while on → spawn while
//! off → re-enable → read the row) rather than a single call in each column.
//!
//! The ungated half of the same fix — that a child is never born below its
//! parent, with the toggle untouched — is asserted beside the code, in
//! `subagent_tool.rs`'s
//! `a_child_is_never_born_below_the_classification_of_its_parent`.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use biorouter::agents::{AgentConfig, TaskConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::Message;
use biorouter::model::ModelConfig;
use biorouter::privacy::{ProviderTier, SessionClassification};
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use rmcp::model::Tool;
use serde_json::json;
use tempfile::TempDir;

/// ⚠ Every mutation of the toggle in this binary goes through one mutex, so two
/// `#[tokio::test]`s cannot interleave their columns. Not `env_lock`'s: its
/// guard is a `std::sync` guard and holding one across an `.await` is not `Send`
/// in a `#[tokio::test]`.
static PRIVACY_TIER_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// ⚠ Drop restores the PREVIOUS value, not `true`: the last test to run must not
/// decide what every test scheduled after it asserts.
struct PrivacyTierGuard {
    prev: bool,
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

impl Drop for PrivacyTierGuard {
    fn drop(&mut self) {
        biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(self.prev);
    }
}

/// ⚠ NOT reentrant — a test holding one guard must `drop` it before taking
/// another. Re-arming mid-test is a bare `set_privacy_tiers_enabled(..)` under
/// the guard already held, which is what the sequence below does.
async fn set_privacy_tiers(on: bool) -> PrivacyTierGuard {
    let _lock = PRIVACY_TIER_LOCK.lock().await;
    let prev = biorouter_mcp::privacy_toggle::privacy_tiers_enabled();
    biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(on);
    PrivacyTierGuard { prev, _lock }
}

/// A provider whose only interesting property is its tier.
struct TieredProvider {
    name: &'static str,
    tier: ProviderTier,
}

#[async_trait]
impl Provider for TieredProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new("tiered", "Tiered", "", "tiered-model", vec![], "", vec![])
    }

    fn get_name(&self) -> &str {
        self.name
    }

    fn tier(&self) -> ProviderTier {
        self.tier
    }

    /// ⚠ A private double must state an affiliation, because every real private
    /// provider does (DR-26). Private + `None` is the one pairing the
    /// vocabulary says cannot exist, and the gate reads it as *unstated* rather
    /// than *unconstrained* — which would drop extensions for a reason this file
    /// is not about. `Local` is DR-26's identity element.
    fn affiliation(&self) -> Option<biorouter::privacy::ModelAffiliation> {
        match self.tier {
            ProviderTier::Private => Some(biorouter::privacy::ModelAffiliation::Local),
            ProviderTier::Public => None,
        }
    }

    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        Ok((
            Message::assistant().with_text("ok"),
            ProviderUsage::new("tiered-model".to_string(), Usage::default()),
        ))
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("tiered-model")
    }
}

fn public_provider() -> Arc<dyn Provider> {
    Arc::new(TieredProvider {
        name: "anthropic",
        tier: ProviderTier::Public,
    })
}

/// **The scenario the finding names, start to finish.**
///
/// 1. Protection ON. A chat reaches a private data source and DR-4's ratchet
///    classifies it Private — the ordinary way a row becomes private.
/// 2. The user takes the documented opt-out and turns protection OFF. The chat
///    is now free to run a public model; that is what the switch is for.
/// 3. The model spawns a subagent, which inherits that public provider. This is
///    the innocent path — no override, no named model, just a delegation.
/// 4. The user turns protection back ON.
///
/// The child's row must still say Private. Nothing in step 4 goes back and
/// looks at rows (AR-7), so if step 3 wrote `public` the lineage of a private
/// chat is public for ever and no later action can correct it.
///
/// ⚠ **The public arm is not decoration.** "Born private" passes vacuously
/// against a stamp that is unconditionally private — which would be its own
/// defect, since a raise is undone only by §12.5's three proofs. Only the pair
/// says the stamp is a `max` over the parent, and not a floor.
///
/// ⚠ **Both spawn paths**, because the fix lands at a helper both of them call
/// and a single-path test would pass with one of them rewired. It is also what
/// says the raise has live callers on the paths a model actually reaches: the
/// entry point here is `handle_subagent_tool`, which `Agent::dispatch` calls,
/// not the helper under test.
#[tokio::test]
async fn a_child_spawned_while_the_switch_is_off_still_carries_its_parents_classification() {
    for (parent_tier, expected_child, background) in [
        (
            SessionClassification::Private,
            SessionClassification::Private,
            false,
        ),
        (
            SessionClassification::Private,
            SessionClassification::Private,
            true,
        ),
        (
            SessionClassification::Public,
            SessionClassification::Public,
            false,
        ),
        (
            SessionClassification::Public,
            SessionClassification::Public,
            true,
        ),
    ] {
        // ---- 1. Protection ON: the parent is classified. -------------------
        let guard = set_privacy_tiers(true).await;

        let dir = TempDir::new().unwrap();
        let sm = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let config = AgentConfig::new(
            sm.clone(),
            Arc::new(PermissionManager::new(dir.path().to_path_buf())),
            None,
            BioRouterMode::Auto,
        );
        let parent = sm
            .create_session(
                dir.path().to_path_buf(),
                "the chat".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        if parent_tier.is_private() {
            sm.update(&parent.id)
                .raise_privacy(parent_tier, "mcp:ucsfomopagent")
                .apply()
                .await
                .unwrap();
        }
        assert_eq!(
            sm.get_session(&parent.id, false)
                .await
                .unwrap()
                .privacy_tier,
            parent_tier,
            "the fixture failed to classify the parent; every assertion below would be vacuous"
        );

        // ---- 2. The documented opt-out. ------------------------------------
        // A bare setter under the guard already held: `set_privacy_tiers` is not
        // reentrant and taking a second one here would deadlock.
        biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(false);
        assert!(!biorouter::privacy::privacy_tiers_enabled());

        // ---- 3. The innocent path: an inheriting spawn. ---------------------
        // No `settings` override — the child is handed the parent's own public
        // `Arc`, so its capability floor is Public. With the switch off, neither
        // spawn refusal fires, which is exactly what the switch promises.
        let task_config = TaskConfig::new(public_provider(), &parent.id, dir.path(), vec![]);
        let overrides = HashMap::from([(
            "BIOROUTER_SUBAGENT_BACKGROUND".to_string(),
            background.to_string(),
        )]);
        let _ = biorouter::config::with_config_overrides(overrides, async {
            biorouter::agents::subagent_tool::handle_subagent_tool(
                &config,
                json!({
                    "instructions": "summarise the cohort",
                    "background": background,
                }),
                task_config,
                HashMap::new(),
                dir.path().to_path_buf(),
                None,
            )
            .result
            .await
        })
        .await;

        // ---- 4. Protection back ON. ----------------------------------------
        biorouter_mcp::privacy_toggle::set_privacy_tiers_enabled(true);

        // ⚠ `list_sessions()` would find nothing here, twice over: it filters to
        // `user`/`scheduled` rows and INNER JOINs `messages`, and a child whose
        // provider bind is refused never records one. The summaries reader with
        // both flags on is the one projection that returns an empty `sub_agent`
        // row — and it carries `privacy_tier`.
        let children: Vec<_> = sm
            .list_session_summaries(50, 0, true, true)
            .await
            .unwrap()
            .into_iter()
            .filter(|s| s.parent_session_id.as_deref() == Some(parent.id.as_str()))
            .collect();
        assert_eq!(
            children.len(),
            1,
            "{parent_tier:?} background={background}: the spawn did not create exactly one child row"
        );
        assert_eq!(
            children[0].privacy_tier, expected_child,
            "{parent_tier:?} background={background}: a row minted while the switch was off cannot be \
             corrected by turning it back on (AR-7): the opt-out laundered the parent's \
             classification"
        );
        // And re-enabling did not retro-classify anything: the parent is where
        // it was, so the row above is inheritance and not a backfill.
        assert_eq!(
            sm.get_session(&parent.id, false)
                .await
                .unwrap()
                .privacy_tier,
            parent_tier
        );

        drop(guard);
    }
}
