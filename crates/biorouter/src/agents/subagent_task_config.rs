use crate::agents::session_skills::SessionSkillOverride;
use crate::agents::ExtensionConfig;
use crate::decision::DecisionService;
use crate::privacy::{CallCapability, SessionClassification};
use crate::providers::base::Provider;
use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Default maximum number of turns for task execution
pub const DEFAULT_SUBAGENT_MAX_TURNS: usize = 25;

/// Environment variable name for configuring max turns
pub const BIOROUTER_SUBAGENT_MAX_TURNS_ENV_VAR: &str = "BIOROUTER_SUBAGENT_MAX_TURNS";

/// Configuration for task execution with all necessary dependencies
///
/// A regular provider is initially the parent's same `Arc<dyn Provider>`, which
/// preserves live provider-local state. Before the child row is created,
/// `subagent_tool::apply_settings_overrides` replaces a lead/worker composite
/// with a reconstructed per-session instance. Its durable recipe copies the
/// parent's current route and both component configurations, while a fresh
/// binding generation and independent counters make each session row
/// authoritative after that point.
#[derive(Clone)]
pub struct TaskConfig {
    pub provider: Arc<dyn Provider>,
    pub parent_session_id: String,
    pub parent_working_dir: PathBuf,
    pub extensions: Vec<ExtensionConfig>,
    pub max_turns: Option<usize>,
    /// Issue #56 §8.2: the classification the child's row is BORN with — the
    /// floor established by the capability of the provider the child actually
    /// resolved, not by the parent's classification.
    ///
    /// Established by `subagent_tool::apply_settings_overrides`, which both
    /// spawn paths run **before** `create_subagent_session`; the value seeded by
    /// [`TaskConfig::new`] is never the one that reaches a row. Public is the
    /// seed because it is what the `sessions.privacy_tier` column defaults to
    /// and because the stamp is a `raise_privacy` — a seed of Private would
    /// permanently over-classify any child whose stamp were ever skipped, and a
    /// ratchet is not revisited.
    pub privacy_tier: SessionClassification,
    //
    // There is deliberately NO downgrade-confirmation flag on this struct
    // (DR-19), and its former name is not written here either — a Step 5 gate
    // greps the crate for it and expects silence, so that a re-add is visible
    // in review rather than buried in prose that has always mentioned it.
    // A private parent naming a public model for its child is refused in
    // `subagent_tool::apply_settings_overrides`, not flagged: the field this
    // struct used to carry had no reader anywhere in the tree — no surface
    // rendered it, no handler branched on it — and a field nothing reads is
    // indistinguishable, in review and in a later audit, from a control that
    // exists. If a future ruling reinstates an approval, it arrives together
    // with its consumer, not before one.
    //
    /// Issue #56 §8.2: private extensions the tier filter removed from the
    /// child's inherited set, by name, so the drop can be reported in the tool
    /// result. A capability that vanishes silently is one the model will keep
    /// planning around.
    pub dropped_private_extensions: Vec<String>,
    /// Issue #56 Task 48 (DR-26): extensions the AFFILIATION filter removed,
    /// as `(name, the warning)`.
    ///
    /// ⚠ **A second list, not an addition to the one above, because the two
    /// drops have different reasons and the parent is told which.** The tier
    /// note says "this subagent is running on a public model" — which is a lie
    /// about an affiliation drop, where the child is Private and the boundary
    /// crossed is institutional. DR-26 requires a statement specific enough to
    /// act on, so the warning is carried rather than recomposed from the name.
    pub dropped_cross_affiliation_extensions: Vec<(String, String)>,
    /// The decision service the parent's agent asks, when a test or an
    /// embedder installed one of its own (`Agent::set_decision_service`). The
    /// child's agent is given the same one, so its monitors and its `decide`
    /// calls go where the parent's go. `None` means the process's service,
    /// which is what a child asks without being told.
    pub decision_service: Option<Arc<DecisionService>>,
    /// The capability the spawning call was admitted on (the parent turn's,
    /// `Agent::sample_turn_call`), which judges `add_extensions` with the one
    /// agent enable gate (`privacy::refusal::extension_enable_refusal`), the
    /// way `workspace_set_tools` judges its own adds. `None` (a coding-agent
    /// bridge spawn, a test, an embedder that built the config itself) means
    /// the child may be given only extensions the parent already holds.
    pub spawn_capability: Option<CallCapability>,
    /// What this spawn asked to change about the extensions the child
    /// inherits (`add_extensions`, `remove_extensions`), resolved and gated at
    /// the spawn tool's entry and applied in `subagent_tool::judge_child`
    /// after the `extensions` narrowing and BEFORE the tier and affiliation
    /// filter, so an added extension is filtered exactly like an inherited one.
    pub extension_changes: ChildExtensionChanges,
    /// The child's own skill and Context choices (`workspace_skills/v1`):
    /// the parent chat's, copied, with this spawn's `add_skills` and
    /// `remove_skills` merged in. Stamped on the child's row at birth, in the
    /// statement that links it to its parent, so its first read of its skills
    /// finds them. Empty means the machine-wide answer, as for any chat.
    pub skill_override: SessionSkillOverride,
}

/// One spawn's changes to the extensions a child inherits from its parent's
/// live roster. See [`TaskConfig::extension_changes`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChildExtensionChanges {
    /// Configs to give the child on top of what it inherits.
    pub add: Vec<ExtensionConfig>,
    /// Manager keys (`extension_manager::normalize(&config.key())`) to take
    /// away. Applied before `add`, so a name in both ends up present.
    pub remove: Vec<String>,
}

impl ChildExtensionChanges {
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty()
    }
}

impl fmt::Debug for TaskConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TaskConfig")
            .field("provider", &"<dyn Provider>")
            .field("parent_session_id", &self.parent_session_id)
            .field("parent_working_dir", &self.parent_working_dir)
            .field("max_turns", &self.max_turns)
            .field("extensions", &self.extensions)
            .field("privacy_tier", &self.privacy_tier)
            .field(
                "dropped_private_extensions",
                &self.dropped_private_extensions,
            )
            .field(
                "dropped_cross_affiliation_extensions",
                &self.dropped_cross_affiliation_extensions,
            )
            .field(
                "decision_service",
                &self
                    .decision_service
                    .as_ref()
                    .map(|_| "<custom DecisionService>"),
            )
            .field("spawn_capability", &self.spawn_capability)
            .field("extension_changes", &self.extension_changes)
            .field("skill_override", &self.skill_override)
            .finish()
    }
}

impl TaskConfig {
    pub fn new(
        provider: Arc<dyn Provider>,
        parent_session_id: &str,
        parent_working_dir: &Path,
        extensions: Vec<ExtensionConfig>,
    ) -> Self {
        Self {
            provider,
            parent_session_id: parent_session_id.to_owned(),
            parent_working_dir: parent_working_dir.to_owned(),
            extensions,
            max_turns: Some(
                env::var(BIOROUTER_SUBAGENT_MAX_TURNS_ENV_VAR)
                    .ok()
                    .and_then(|val| val.parse::<usize>().ok())
                    .unwrap_or(DEFAULT_SUBAGENT_MAX_TURNS),
            ),
            // Seeds, not decisions. `subagent_tool::apply_settings_overrides`
            // establishes both, and it runs before the child's row exists.
            // Deliberately NOT `floor(provider.tier())` here: `floor` is the one
            // crossing between the capability and classification lattices and its
            // caller set is asserted by a repo-grep audit, so the crossing lives
            // at the single site that decides it.
            privacy_tier: SessionClassification::Public,
            dropped_private_extensions: Vec::new(),
            dropped_cross_affiliation_extensions: Vec::new(),
            decision_service: None,
            spawn_capability: None,
            extension_changes: ChildExtensionChanges::default(),
            skill_override: SessionSkillOverride::default(),
        }
    }

    /// The same config, with the child asking `service` for decisions (its
    /// monitors and its `decide` calls). `None` keeps the process's service.
    pub fn with_decision_service(mut self, service: Option<Arc<DecisionService>>) -> Self {
        self.decision_service = service;
        self
    }

    /// The same config, judging `add_extensions` with `capability`: the
    /// capability the spawning call was admitted on.
    pub fn with_spawn_capability(mut self, capability: Option<CallCapability>) -> Self {
        self.spawn_capability = capability;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source with no models: the service is only carried here, never asked.
    struct NoModels;

    #[async_trait::async_trait]
    impl crate::decision::DecisionModelSource for NoModels {
        async fn list(&self) -> Vec<crate::decision::DecisionModelEntry> {
            Vec::new()
        }
        async fn open(
            &self,
            wanted: &crate::decision::DecisionModelRef,
        ) -> crate::decision::DecisionResult<Arc<dyn crate::decision::DecisionModel>> {
            Err(crate::decision::DecisionError::NoModel(format!(
                "{}/{} is not installed",
                wanted.provider, wanted.model
            )))
        }
    }

    fn config() -> TaskConfig {
        // A replaying provider with no cassette: never called here.
        let cassette = std::env::temp_dir().join("biorouter-task-config-test-no-cassette.json");
        let provider: Arc<dyn Provider> = Arc::new(
            crate::providers::testprovider::TestProvider::new_replaying(
                cassette.to_string_lossy().to_string(),
            )
            .expect("an empty replaying test provider"),
        );
        TaskConfig::new(provider, "parent-1", Path::new("."), Vec::new())
    }

    /// A child asks the parent's custom decision service, not the process's,
    /// only when it is handed one; `new` leaves it to the process.
    #[test]
    fn the_decision_service_is_carried_only_when_given() {
        let plain = config();
        assert!(plain.decision_service.is_none());
        assert!(format!("{plain:?}").contains("decision_service: None"));

        let service = Arc::new(DecisionService::new(Arc::new(NoModels)));
        let carried = config().with_decision_service(Some(Arc::clone(&service)));
        let got = carried
            .decision_service
            .as_ref()
            .expect("the service rides on the config");
        assert!(Arc::ptr_eq(got, &service));
        assert!(format!("{carried:?}").contains("<custom DecisionService>"));
        // A clone (the background path clones before it moves) keeps it.
        assert!(Arc::ptr_eq(
            carried.clone().decision_service.as_ref().unwrap(),
            &service
        ));

        assert!(carried
            .with_decision_service(None)
            .decision_service
            .is_none());
    }
}
