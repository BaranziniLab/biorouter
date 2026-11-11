//! DR-33: a chat's **decision models** count toward its capability, and its
//! least private model sets it.
//!
//! A chat runs on its bound language model and on the standing decision models
//! it names (a default, its enabled monitors, the gates and phase monitors of
//! its open phases). The owner's ruling of 2026-11-10 is that the least private
//! of them decides what the chat may reach: Versa plus a Baseten monitor is a
//! public chat, Versa plus a local Ollama monitor stays private.
//!
//! This module is the fold, and nothing else. It is pure: it reads a session
//! row (or the same inputs before they are stored), resolves each standing name
//! to the tier its provider declares, and returns the [`DecisionAxes`] the
//! decision-bound provider (`providers::decision_bound`) folds into the bound
//! model's own tier and affiliation. No I/O, no network, no read of the master
//! switch. The only configuration read is `OLLAMA_HOST`, through the cached
//! `Config::global()` read every provider tier already does.
//!
//! # The three rules that keep it safe
//!
//! * **The inert rule** (design §1.4). A name on a chat classified Private is
//!   not a model it uses: Gate H refuses every public decision request on such
//!   a chat, so the fold drops every input there. On a Private chat the result
//!   is always the identity, and every gate's outcome is exactly what it was
//!   before DR-33.
//! * **A lowering is a person's act** (design §1.8). A public name lowers
//!   capability only while the chat carries a valid `public_basis.v1`: the
//!   record `SessionManager::write_public_basis` writes, and only it, after a
//!   person accepted the lowering on a chat whose content was all produced at
//!   Public capability. A public name with no valid basis is inert.
//! * **Doubt never lowers** (design §1.5). An unknown provider, an unreadable
//!   record, an Ollama host that does not resolve, a basis or ceiling that does
//!   not parse or names another chat: each resolves to *not lowering*. That is
//!   an availability loss (Gate H refuses the doubtful model per call), never a
//!   disclosure.
//!
//! The fold never returns Private from a name. A private name changes nothing
//! (`least(x, Private) = x`), so it contributes the identity, and the only value
//! a name can contribute is Public.
//!
//! Computed at the bind, at Gate B, after an admitted lowering, and at spawn;
//! every other caller only explains or predicts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::affiliation::ModelAffiliation;
use super::{ProviderTier, SessionClassification};
use crate::agents::orchestration_plan::{OrchestrationPlan, PLAN_STATE, PLAN_VERSION};
use crate::decision::settings::{
    DecisionSettings, DECISION_SETTINGS_STATE, DECISION_SETTINGS_VERSION,
};
use crate::decision::DecisionModelRef;
use crate::providers::decision::catalog::{BASETEN_DECISION_PROVIDER, OLLAMA_DECISION_PROVIDER};
use crate::session::extension_data::ExtensionData;
use crate::session::session_manager::{Session, SessionType};

/// The `extension_data` key of the person's acceptance of a lowering.
pub const PUBLIC_BASIS_STATE: &str = "public_basis";
pub const PUBLIC_BASIS_VERSION: &str = "v1";
/// The `extension_data` key of a subagent's spawn-time ceiling.
pub const PRIVACY_CEILING_STATE: &str = "privacy_ceiling";
pub const PRIVACY_CEILING_VERSION: &str = "v1";
/// The `extension_data` key of the note left when public decision models were
/// turned off for a chat (declassification, a branch, a copy, an import).
/// `DecisionSettings` refuses unknown fields, so the note cannot live inside it.
pub const DECISION_NOTICE_STATE: &str = "decision_notice";
pub const DECISION_NOTICE_VERSION: &str = "v1";

/// What the note says when a derived chat (branch, copy, import) had its public
/// decision models turned off.
pub const DERIVED_NOTICE: &str =
    "Turned off when this chat was copied: a public decision model has to be chosen again for \
     this chat.";
/// What the note says when declassification turned them off.
pub const DECLASSIFIED_NOTICE: &str = "Turned off when this chat was made public.";

// ---------------------------------------------------------------------------
// The stored records.
// ---------------------------------------------------------------------------

/// Who accepted a lowering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BasisAuthor {
    /// A person, at a door that checked they acted (the user-action proof, a
    /// confirmation typed at a terminal).
    Person,
    /// A schedule armed by a proven door with the person's acceptance.
    Schedule,
}

/// `public_basis.v1`: the record that a person made this chat public by naming
/// a decision model. Written only by `SessionManager::write_public_basis`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PublicBasis {
    /// The chat it was written for. A basis copied onto another row names the
    /// wrong chat and is therefore not valid there.
    pub session_id: String,
    pub by: BasisAuthor,
    /// The door that wrote it (`decision-settings`, `phase-gate`,
    /// `agent-start`, `cli`, `schedule`), for the audit trail.
    pub door: String,
    pub at: DateTime<Utc>,
    /// The public models the person accepted, as `provider/model`. Audit only:
    /// validity is a property of the chat, not of each name.
    #[serde(default)]
    pub models: Vec<String>,
}

impl PublicBasis {
    /// `models` is kept in order with each model once: two sources naming one
    /// model (a monitor and a gate both on `joey-1`) are one accepted model
    /// (live QA M3-5).
    pub fn new(
        session_id: impl Into<String>,
        by: BasisAuthor,
        door: impl Into<String>,
        models: Vec<String>,
    ) -> Self {
        let mut seen = std::collections::HashSet::new();
        let models = models
            .into_iter()
            .filter(|model| seen.insert(model.clone()))
            .collect();
        Self {
            session_id: session_id.into(),
            by,
            door: door.into(),
            at: Utc::now(),
            models,
        }
    }
}

/// `privacy_ceiling.v1`: a subagent born under a public parent runs at its
/// parent's tier (design §3.4). Written only at spawn, in the statement that
/// sets the child's classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyCeiling {
    pub tier: ProviderTier,
    /// The parent it was written under. Valid only on a subagent whose
    /// `parent_session_id` is this id, so a branch or copy of the child does not
    /// inherit it.
    pub from_session: String,
    /// Why the parent was public, for the child's refusals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lowered_by: Option<LoweredBy>,
}

impl PrivacyCeiling {
    /// The ceiling a child of `parent_id` carries when its parent's effective
    /// tier at the spawn is Public.
    pub fn public_from(parent_id: impl Into<String>, lowered_by: Option<LoweredBy>) -> Self {
        Self {
            tier: ProviderTier::Public,
            from_session: parent_id.into(),
            lowered_by,
        }
    }

    /// Put this ceiling into a new child's `extension_data`, to be written in
    /// the SAME statement that sets the child's classification (design §3.4):
    /// `sm.update(child).raise_privacy(..).extension_data(data).apply()`.
    pub fn stamp(&self, data: &mut ExtensionData) -> anyhow::Result<()> {
        data.set_extension_state(
            PRIVACY_CEILING_STATE,
            PRIVACY_CEILING_VERSION,
            serde_json::to_value(self)?,
        );
        Ok(())
    }

    /// Whether this ceiling names `caller` as the parent it was written under.
    pub fn names(&self, caller: &str) -> bool {
        self.from_session == caller
    }
}

/// `decision_notice.v1`: what was turned off, and why, so the Monitors dialog
/// can say so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionNotice {
    pub text: String,
    /// Ids of the monitors that were disabled.
    #[serde(default)]
    pub monitors: Vec<String>,
    /// The public default model that was cleared, as `provider/model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleared_default: Option<String>,
    pub at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// The cause of a lowering, in words.
// ---------------------------------------------------------------------------

/// Which standing name a lowering came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NameSource {
    /// `decision_settings.v1.default_model`.
    Default,
    /// An enabled monitor of the chat.
    Monitor,
    /// The gate of an open phase of the chat's plan.
    Gate,
    /// A monitor of an open phase, which the subagents spawned into it carry.
    PhaseMonitor,
    /// The spawn-time ceiling: the chat is part of a public parent's work.
    Parent,
}

/// Where a public decision model runs, for the sentence.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case", tag = "kind", content = "host")]
pub enum HostLabel {
    Baseten,
    ThisComputer,
    /// An Ollama server that is not this computer.
    OllamaAt(String),
    /// An Ollama cloud tag, which Ollama forwards to ollama.com.
    OllamaCloud,
}

impl HostLabel {
    /// "a Baseten model", "an Ollama cloud model", …
    pub fn model_noun(&self) -> String {
        match self {
            Self::Baseten => "a Baseten model".to_string(),
            Self::ThisComputer => "a model on this computer".to_string(),
            Self::OllamaAt(host) => format!("a model on the Ollama server at {host}"),
            Self::OllamaCloud => "an Ollama cloud model".to_string(),
        }
    }

    /// "Baseten", "this computer", …
    pub fn place(&self) -> String {
        match self {
            Self::Baseten => "Baseten".to_string(),
            Self::ThisComputer => "this computer".to_string(),
            Self::OllamaAt(host) => format!("the Ollama server at {host}"),
            Self::OllamaCloud => "Ollama's cloud".to_string(),
        }
    }
}

/// The standing name that made a chat public, in the words a refusal uses.
/// Text only: it decides nothing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoweredBy {
    pub source: NameSource,
    /// The monitor's name, the phase's title, or the parent chat's id.
    pub label: String,
    /// The model's own name (`joey-1`).
    pub model: String,
    pub host: HostLabel,
    /// How many other public names the fold also counted, so a sentence that
    /// names this one does not imply it is the only one (and does not offer a
    /// remedy for it alone). Zero on a single name.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub others: usize,
}

fn is_zero(count: &usize) -> bool {
    *count == 0
}

impl LoweredBy {
    /// The model as a person or an agent reads it: a Baseten model by its
    /// alias (`joey-1`, never `decision-models/baseten-joey-2b-latest`), any
    /// other by its id. `model` keeps what was stored, which a command line
    /// or a record may need; every sentence names this (live QA 2026-11-11:
    /// "This chat stays public: decision-models/baseten-joey-2b-latest runs
    /// on Baseten").
    pub fn model_name(&self) -> String {
        match self.host {
            HostLabel::Baseten => {
                crate::providers::decision::catalog::baseten_display_name(&self.model)
            }
            _ => self.model.clone(),
        }
    }

    /// The subject, for a person: `monitor "Done" uses joey-1, a Baseten model`,
    /// and, when the fold counted more, `…, and 2 other public decision models`.
    pub fn person_phrase(&self) -> String {
        let one = self.one_person_phrase();
        match self.others {
            0 => one,
            1 => format!("{one}, and 1 other public decision model"),
            n => format!("{one}, and {n} other public decision models"),
        }
    }

    fn one_person_phrase(&self) -> String {
        let noun = self.host.model_noun();
        match self.source {
            NameSource::Default => format!(
                "its default decision model is {}, {noun}",
                self.model_name()
            ),
            NameSource::Monitor => {
                format!(
                    "monitor \"{}\" uses {}, {noun}",
                    self.label,
                    self.model_name()
                )
            }
            NameSource::Gate => format!(
                "the gate on phase \"{}\" uses {}, {noun}",
                self.label,
                self.model_name()
            ),
            NameSource::PhaseMonitor => format!(
                "subagent monitor \"{}\" uses {}, {noun}",
                self.label,
                self.model_name()
            ),
            NameSource::Parent if self.model.is_empty() => {
                "it is part of the work of a public chat".to_string()
            }
            NameSource::Parent => format!(
                "it is part of the work of a chat that is public because it uses {}, {noun}",
                self.model_name()
            ),
        }
    }

    /// The parenthesis an agent reads: `monitor "Done", joey-1 on Baseten`,
    /// and `…, and 2 more` when the fold counted more.
    pub fn agent_phrase(&self) -> String {
        let one = self.one_agent_phrase();
        match self.others {
            0 => one,
            n => format!("{one}, and {n} more"),
        }
    }

    fn one_agent_phrase(&self) -> String {
        let place = self.host.place();
        match self.source {
            NameSource::Default => {
                format!("default decision model, {} on {place}", self.model_name())
            }
            NameSource::Monitor => format!(
                "monitor \"{}\", {} on {place}",
                self.label,
                self.model_name()
            ),
            NameSource::Gate => format!(
                "gate on phase \"{}\", {} on {place}",
                self.label,
                self.model_name()
            ),
            NameSource::PhaseMonitor => format!(
                "subagent monitor \"{}\", {} on {place}",
                self.label,
                self.model_name()
            ),
            NameSource::Parent if self.model.is_empty() => {
                "the public chat that started this one".to_string()
            }
            NameSource::Parent => format!(
                "the public chat that started this one, {} on {place}",
                self.model_name()
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// The value.
// ---------------------------------------------------------------------------

/// What a chat's standing decision models contribute to its capability. The
/// identity contributes nothing; otherwise it is Public (a counted public name,
/// or a valid public ceiling), with no affiliation, and names the cause.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DecisionAxes {
    tier: Option<ProviderTier>,
    /// Meaningful only while `tier` is `Some`.
    affiliation: Option<ModelAffiliation>,
    lowered_by: Option<LoweredBy>,
}

impl DecisionAxes {
    /// Nothing counted.
    pub fn identity() -> Self {
        Self::default()
    }

    pub fn is_identity(&self) -> bool {
        self.tier.is_none()
    }

    /// `None` is the identity; a name can only ever contribute `Public`.
    pub fn tier(&self) -> Option<ProviderTier> {
        self.tier
    }

    /// The name the sentences use, when the axes are not the identity.
    pub fn lowered_by(&self) -> Option<&LoweredBy> {
        self.lowered_by.as_ref()
    }

    /// `least(inner, axes)` and `composite_affiliation(inner, axes)`, folded
    /// from the same inputs in the same call so the pair stays consistent.
    pub fn apply(
        &self,
        inner_tier: ProviderTier,
        inner_affiliation: Option<ModelAffiliation>,
    ) -> (ProviderTier, Option<ModelAffiliation>) {
        match self.tier {
            None => (inner_tier, inner_affiliation),
            Some(tier) => (
                ProviderTier::least(inner_tier, tier),
                crate::providers::composite_affiliation(inner_affiliation, self.affiliation),
            ),
        }
    }

    /// Whether these axes make a chat bound to `inner_tier` less private than
    /// its model: the one case a refusal names a decision model.
    pub fn lowers(&self, inner_tier: ProviderTier) -> bool {
        inner_tier.is_private() && self.tier == Some(ProviderTier::Public)
    }

    /// Public axes with `cause`, for the tests of the wrapper and the agent.
    #[cfg(test)]
    pub(crate) fn public_for_test(cause: LoweredBy) -> Self {
        let mut axes = Self::identity();
        axes.fold_public(cause);
        axes
    }

    fn fold_public(&mut self, cause: LoweredBy) {
        self.tier = Some(ProviderTier::Public);
        self.affiliation = None;
        match &mut self.lowered_by {
            None => self.lowered_by = Some(LoweredBy { others: 0, ..cause }),
            Some(first) => first.others += 1,
        }
    }
}

// ---------------------------------------------------------------------------
// The inputs.
// ---------------------------------------------------------------------------

/// The chat's decision settings, as the fold sees them.
#[derive(Debug, Clone, Copy)]
pub enum SettingsInput<'a> {
    Absent,
    /// A stored record this build cannot read: no names (doubt never lowers).
    Unreadable,
    Parsed(&'a DecisionSettings),
}

/// Everything the fold reads, borrowed, so a save can predict its own effect
/// over settings that are not stored yet.
#[derive(Debug, Clone, Copy)]
pub struct AxesInputs<'a> {
    pub session_id: &'a str,
    pub classification: SessionClassification,
    pub session_type: SessionType,
    pub parent_session_id: Option<&'a str>,
    pub settings: SettingsInput<'a>,
    /// `None` when the chat has no plan or its plan cannot be read.
    pub plan: Option<&'a OrchestrationPlan>,
    pub basis: Option<&'a PublicBasis>,
    pub ceiling: Option<&'a PrivacyCeiling>,
}

/// A row's stored records, parsed once, which [`AxesInputs`] borrows from.
#[derive(Debug, Clone)]
pub struct RowRecords {
    settings: Result<Option<DecisionSettings>, ()>,
    plan: Option<OrchestrationPlan>,
    basis: Option<PublicBasis>,
    ceiling: Option<PrivacyCeiling>,
}

impl RowRecords {
    /// Parse the four records a fold reads out of `extension_data`. A record
    /// that does not parse is absent (or, for the settings, unreadable).
    pub fn parse(data: &ExtensionData) -> Self {
        let state = |name: &str, version: &str| {
            data.get_extension_state(name, version)
                .filter(|value| !value.is_null())
                .cloned()
        };
        // The same typed deserializer `decision::settings::load` and the monitor
        // engine use, so the fold and monitoring cannot disagree about which
        // names are live.
        let settings = match state(DECISION_SETTINGS_STATE, DECISION_SETTINGS_VERSION) {
            None => Ok(None),
            Some(value) => serde_json::from_value::<DecisionSettings>(value)
                .map(Some)
                .map_err(|_| ()),
        };
        Self {
            settings,
            plan: state(PLAN_STATE, PLAN_VERSION)
                .and_then(|value| serde_json::from_value(value).ok()),
            basis: state(PUBLIC_BASIS_STATE, PUBLIC_BASIS_VERSION)
                .and_then(|value| serde_json::from_value(value).ok()),
            ceiling: state(PRIVACY_CEILING_STATE, PRIVACY_CEILING_VERSION)
                .and_then(|value| serde_json::from_value(value).ok()),
        }
    }

    pub fn settings(&self) -> SettingsInput<'_> {
        match &self.settings {
            Ok(None) => SettingsInput::Absent,
            Ok(Some(settings)) => SettingsInput::Parsed(settings),
            Err(()) => SettingsInput::Unreadable,
        }
    }

    pub fn basis(&self) -> Option<&PublicBasis> {
        self.basis.as_ref()
    }

    pub fn ceiling(&self) -> Option<&PrivacyCeiling> {
        self.ceiling.as_ref()
    }

    pub fn plan(&self) -> Option<&OrchestrationPlan> {
        self.plan.as_ref()
    }

    /// The inputs for `row`, borrowing these records.
    pub fn inputs<'a>(&'a self, row: &'a Session) -> AxesInputs<'a> {
        AxesInputs {
            session_id: &row.id,
            classification: row.privacy_tier,
            session_type: row.session_type,
            parent_session_id: row.parent_session_id.as_deref(),
            settings: self.settings(),
            plan: self.plan(),
            basis: self.basis(),
            ceiling: self.ceiling(),
        }
    }
}

impl<'a> AxesInputs<'a> {
    /// The same inputs with other decision settings: what a save would store.
    pub fn with_settings(self, settings: &'a DecisionSettings) -> Self {
        Self {
            settings: SettingsInput::Parsed(settings),
            ..self
        }
    }

    /// The same inputs with another plan: what a gate save would store.
    pub fn with_plan(self, plan: Option<&'a OrchestrationPlan>) -> Self {
        Self { plan, ..self }
    }

    /// Whether the basis is valid for this chat: present, parsed, naming this
    /// chat, on a chat classified Public.
    pub fn basis_valid(&self) -> bool {
        !self.classification.is_private()
            && self
                .basis
                .is_some_and(|basis| basis.session_id == self.session_id)
    }

    /// Whether the ceiling is valid for this chat: on the subagent it was
    /// written for, classified Public.
    pub fn ceiling_valid(&self) -> bool {
        !self.classification.is_private()
            && self.session_type == SessionType::SubAgent
            && self.ceiling.is_some_and(|ceiling| {
                self.parent_session_id
                    .is_some_and(|parent| ceiling.from_session == parent)
            })
    }
}

// ---------------------------------------------------------------------------
// The standing names.
// ---------------------------------------------------------------------------

/// One name the chat stands on (design §1.1), before its tier is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandingName {
    pub source: NameSource,
    pub label: String,
    pub model: DecisionModelRef,
}

/// Every standing name of the chat, in a fixed order: the default, the enabled
/// monitors, the gates of open phases, the monitors of open phases.
///
/// Not counted: a disabled monitor, a monitor or gate with no model (it resolves
/// at call time and follows the tier, §1.2), a closed phase's gate or monitors,
/// a phase monitor whose model does not parse, and every name of an unreadable
/// settings record.
pub fn standing_names(inputs: &AxesInputs<'_>) -> Vec<StandingName> {
    let mut names = Vec::new();
    if let SettingsInput::Parsed(settings) = inputs.settings {
        if let Some(model) = &settings.default_model {
            names.push(StandingName {
                source: NameSource::Default,
                label: "default".to_string(),
                model: model.clone(),
            });
        }
        for monitor in settings.monitors.iter().filter(|m| m.enabled) {
            if let Some(model) = &monitor.model {
                names.push(StandingName {
                    source: NameSource::Monitor,
                    label: monitor.name.clone(),
                    model: model.clone(),
                });
            }
        }
    }
    if let Some(plan) = inputs.plan {
        let open = || plan.phases.iter().filter(|phase| !phase.status.is_closed());
        for phase in open() {
            if let Some(model) = phase.gate.as_ref().and_then(|gate| gate.model.as_ref()) {
                names.push(StandingName {
                    source: NameSource::Gate,
                    label: phase.title.clone(),
                    model: model.clone(),
                });
            }
        }
        for phase in open() {
            for monitor in &phase.monitors {
                let Some(model) = monitor
                    .model
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .and_then(DecisionModelRef::parse)
                else {
                    continue;
                };
                names.push(StandingName {
                    source: NameSource::PhaseMonitor,
                    label: monitor.name.clone(),
                    model,
                });
            }
        }
    }
    names
}

/// What a decision model's provider declares, without opening it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredName {
    pub tier: ProviderTier,
    pub affiliation: Option<ModelAffiliation>,
    pub host: HostLabel,
}

/// The declaration the fold uses: `providers::decision::chat::declared_axes`,
/// the pure function the decision adapter's own tier comes from. `None` is
/// doubt (an unknown provider, an Ollama host that does not resolve), which is
/// not counted.
///
/// ⚠ A model renamed from a cloud model with `ollama cp` declares Private here
/// (no network); the adapter declares Public when it is opened and Gate H
/// refuses it. Not lowering is the safe direction for that residual.
pub fn declared_name(model: &DecisionModelRef) -> Option<DeclaredName> {
    use crate::providers::decision::chat::{declared_axes, DecisionChatKind};
    match model.provider.as_str() {
        BASETEN_DECISION_PROVIDER => {
            let (tier, affiliation) = declared_axes(DecisionChatKind::Baseten, &model.model);
            Some(DeclaredName {
                tier,
                affiliation,
                host: HostLabel::Baseten,
            })
        }
        OLLAMA_DECISION_PROVIDER => {
            if crate::providers::utils::is_ollama_cloud_model(&model.model) {
                let (tier, affiliation) = declared_axes(DecisionChatKind::Ollama, &model.model);
                return Some(DeclaredName {
                    tier,
                    affiliation,
                    host: HostLabel::OllamaCloud,
                });
            }
            // `declared_axes` reads an address that does not resolve as Public,
            // which is right for the adapter (it claims nothing) and wrong for
            // the fold (doubt never lowers). So the address is asked first.
            let base = crate::providers::decision::catalog::ollama_base_url().ok()?;
            let (tier, affiliation) = declared_axes(DecisionChatKind::Ollama, &model.model);
            let host = if tier.is_private() {
                HostLabel::ThisComputer
            } else {
                HostLabel::OllamaAt(
                    base.host_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| base.to_string()),
                )
            };
            Some(DeclaredName {
                tier,
                affiliation,
                host,
            })
        }
        _ => None,
    }
}

/// Every standing name that declares Public, as the cause it would be. Counted
/// whether or not a basis is valid: this is what a save is about to make
/// standing, not what the chat may reach.
pub fn public_standing_names(inputs: &AxesInputs<'_>) -> Vec<LoweredBy> {
    public_standing_names_with(inputs, &declared_name)
}

pub fn public_standing_names_with(
    inputs: &AxesInputs<'_>,
    declare: &dyn Fn(&DecisionModelRef) -> Option<DeclaredName>,
) -> Vec<LoweredBy> {
    standing_names(inputs)
        .into_iter()
        .filter_map(|name| {
            let declared = declare(&name.model)?;
            (!declared.tier.is_private()).then_some(LoweredBy {
                source: name.source,
                label: name.label,
                model: name.model.model,
                host: declared.host,
                others: 0,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The fold.
// ---------------------------------------------------------------------------

/// The fold over inputs that may not be stored yet (spawn, a save's effect).
pub fn fold(inputs: &AxesInputs<'_>) -> DecisionAxes {
    fold_with(inputs, &declared_name)
}

/// [`fold`] with the declaration supplied, for tests and for a caller that has
/// already resolved its names.
pub fn fold_with(
    inputs: &AxesInputs<'_>,
    declare: &dyn Fn(&DecisionModelRef) -> Option<DeclaredName>,
) -> DecisionAxes {
    fold_inner(inputs, declare, inputs.basis_valid())
}

/// The fold as it would be once this save's basis is written: a public name is
/// counted on a Public chat whether or not a basis is stored yet. For
/// predicting a save's effect only; never installed.
pub fn fold_assuming_basis(inputs: &AxesInputs<'_>) -> DecisionAxes {
    fold_assuming_basis_with(inputs, &declared_name)
}

pub fn fold_assuming_basis_with(
    inputs: &AxesInputs<'_>,
    declare: &dyn Fn(&DecisionModelRef) -> Option<DeclaredName>,
) -> DecisionAxes {
    fold_inner(inputs, declare, !inputs.classification.is_private())
}

fn fold_inner(
    inputs: &AxesInputs<'_>,
    declare: &dyn Fn(&DecisionModelRef) -> Option<DeclaredName>,
    count_public: bool,
) -> DecisionAxes {
    // The inert rule: a Private chat's names are not models it uses.
    if inputs.classification.is_private() {
        return DecisionAxes::identity();
    }
    let mut axes = DecisionAxes::identity();
    if count_public {
        for cause in public_standing_names_with(inputs, declare) {
            axes.fold_public(cause);
        }
    }
    if inputs.ceiling_valid() {
        if let Some(ceiling) = inputs.ceiling {
            if !ceiling.tier.is_private() {
                let cause = ceiling.lowered_by.clone().unwrap_or(LoweredBy {
                    source: NameSource::Parent,
                    label: ceiling.from_session.clone(),
                    model: String::new(),
                    host: HostLabel::Baseten,
                    others: 0,
                });
                axes.fold_public(LoweredBy {
                    source: NameSource::Parent,
                    others: 0,
                    ..cause
                });
            }
        }
    }
    axes
}

/// Whether `row` is a subagent carrying a spawn ceiling written under
/// `caller_id`, its parent (design §3.4). What
/// [`crate::privacy::crossing::needs_disclosure_to`] asks of a write's target.
/// Read whatever the row's classification, because the question is who the
/// child was born under, not what it may reach.
pub fn ceiling_names_caller(row: &Session, caller_id: &str) -> bool {
    row.session_type == SessionType::SubAgent
        && row.parent_session_id.as_deref() == Some(caller_id)
        && RowRecords::parse(&row.extension_data)
            .ceiling()
            .is_some_and(|ceiling| ceiling.names(caller_id))
}

/// The fold of a stored row: its classification, type, parent, and the four
/// `extension_data` records. No I/O, no flag read.
pub fn from_row(row: &Session) -> DecisionAxes {
    let records = RowRecords::parse(&row.extension_data);
    fold(&records.inputs(row))
}

// ---------------------------------------------------------------------------
// Eligibility and the tier effect of a save.
// ---------------------------------------------------------------------------

/// Whether a chat may be lowered now (design §1.8). Mirrors the condition
/// `SessionManager::write_public_basis` checks again inside its write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eligibility {
    /// Classified Public and holds no message.
    Empty,
    /// Classified Public because the person declassified it.
    Declassified,
    /// Classified Public with a valid basis: everything since the lowering was
    /// produced at Public capability.
    AlreadyLowered,
    NotEligible,
}

impl Eligibility {
    pub fn is_eligible(self) -> bool {
        !matches!(self, Self::NotEligible)
    }
}

/// The eligibility of a stored row. `row.message_count` must be the row's real
/// count, which `get_session` always fills.
pub fn eligibility(row: &Session) -> Eligibility {
    let records = RowRecords::parse(&row.extension_data);
    eligibility_of(
        &records.inputs(row),
        row.message_count,
        row.privacy_reason.as_deref(),
    )
}

pub fn eligibility_of(
    inputs: &AxesInputs<'_>,
    message_count: usize,
    privacy_reason: Option<&str>,
) -> Eligibility {
    if inputs.classification.is_private() {
        Eligibility::NotEligible
    } else if message_count == 0 {
        Eligibility::Empty
    } else if privacy_reason == Some(super::declassify::DECLASSIFIED_BY_USER) {
        Eligibility::Declassified
    } else if inputs.basis_valid() {
        Eligibility::AlreadyLowered
    } else {
        Eligibility::NotEligible
    }
}

/// What a save does to a chat's tier (design §3.2). Computed from two folds and
/// never from the slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum TierEffect {
    None,
    BecomesPublic,
    StaysPublic,
    BecomesPrivateAtNextMessage,
    RefusedNotEligible,
    RefusedPrivate,
}

impl TierEffect {
    /// Whether the save may be written at all (an agent door additionally
    /// refuses [`Self::moves_the_tier`]).
    pub fn is_refusal(self) -> bool {
        matches!(self, Self::RefusedNotEligible | Self::RefusedPrivate)
    }

    /// Whether the save changes whether the chat is private, which only a
    /// person may do.
    pub fn moves_the_tier(self) -> bool {
        matches!(
            self,
            Self::BecomesPublic | Self::BecomesPrivateAtNextMessage
        )
    }

    /// Whether an agent's save with this effect is refused (design §1.8, §3.14).
    pub fn refused_to_an_agent(self) -> bool {
        self.moves_the_tier() || self.is_refusal()
    }
}

/// What a save's effect asks the writer to do with `public_basis.v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasisIntent {
    /// Write (or refresh) the basis, in the same statement as the save.
    Set,
    /// Remove it, in the same statement as the save.
    Clear,
    /// Leave it as it is.
    Keep,
}

/// The tier effect of changing `before` into `after` on a chat bound to
/// `bound_model_tier` (the INNER provider's tier, never the slot's).
pub fn tier_effect(
    bound_model_tier: ProviderTier,
    before: &AxesInputs<'_>,
    after: &AxesInputs<'_>,
    eligibility: Eligibility,
) -> TierEffect {
    tier_effect_with(bound_model_tier, before, after, eligibility, &declared_name)
}

pub fn tier_effect_with(
    bound_model_tier: ProviderTier,
    before: &AxesInputs<'_>,
    after: &AxesInputs<'_>,
    eligibility: Eligibility,
    declare: &dyn Fn(&DecisionModelRef) -> Option<DeclaredName>,
) -> TierEffect {
    let old: Vec<LoweredBy> = public_standing_names_with(before, declare);
    let added = public_standing_names_with(after, declare)
        .into_iter()
        .any(|name| !old.contains(&name));
    if before.classification.is_private() {
        // Asks only what the save changes: an inert name already stored does
        // not refuse an unrelated edit.
        return if added {
            TierEffect::RefusedPrivate
        } else {
            TierEffect::None
        };
    }
    if !bound_model_tier.is_private() {
        // A public model's chat is public whatever it names.
        return TierEffect::None;
    }
    let lowered_before = fold_with(before, declare).lowers(bound_model_tier);
    if lowered_before {
        let lowered_after = fold_assuming_basis_with(after, declare).lowers(bound_model_tier);
        return if lowered_after {
            TierEffect::StaysPublic
        } else {
            TierEffect::BecomesPrivateAtNextMessage
        };
    }
    // Not lowered before: only a public name this save makes standing can lower
    // the chat. A stranded one it merely keeps stays inert.
    if !added {
        return TierEffect::None;
    }
    if eligibility.is_eligible() {
        TierEffect::BecomesPublic
    } else {
        TierEffect::RefusedNotEligible
    }
}

/// What the writer does with the basis for an admitted save.
///
/// `after_has_public` is whether the stored result names any public decision
/// model. A public model's chat that is eligible gets its basis silently (so a
/// later switch to a private model keeps it public); any save that leaves no
/// public name clears it.
///
/// A basis is asked for only where one can validly be written (the chat is
/// eligible, which `write_public_basis` checks again). A `staysPublic` save on
/// a chat that is public only through its spawn ceiling, and has messages,
/// keeps what is there: the ceiling is what keeps it public, no basis could be
/// written, and asking for one refused an ordinary edit (a local monitor) with
/// "this chat has started".
pub fn basis_intent(
    effect: TierEffect,
    bound_model_tier: ProviderTier,
    eligibility: Eligibility,
    after_has_public: bool,
) -> BasisIntent {
    match effect {
        TierEffect::BecomesPublic => BasisIntent::Set,
        TierEffect::StaysPublic if eligibility.is_eligible() => BasisIntent::Set,
        TierEffect::StaysPublic => BasisIntent::Keep,
        TierEffect::BecomesPrivateAtNextMessage => BasisIntent::Clear,
        TierEffect::RefusedNotEligible | TierEffect::RefusedPrivate => BasisIntent::Keep,
        TierEffect::None if !after_has_public => BasisIntent::Clear,
        TierEffect::None if !bound_model_tier.is_private() && eligibility.is_eligible() => {
            BasisIntent::Set
        }
        TierEffect::None => BasisIntent::Keep,
    }
}

// ---------------------------------------------------------------------------
// Derived rows and declassification.
// ---------------------------------------------------------------------------

/// What turning a chat's public decision models off did, and would do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PublicNamesOff {
    /// `(monitor id, monitor name, model label)` for each disabled monitor.
    pub monitors: Vec<(String, String, String)>,
    /// The public default model that was cleared, as `provider/model`.
    pub cleared_default: Option<String>,
    /// `(phase title, model label)` for each open phase whose gate is public.
    /// Gates have no paused state, so these are listed, never changed.
    pub open_public_gates: Vec<(String, String)>,
}

impl PublicNamesOff {
    pub fn is_empty(&self) -> bool {
        self.monitors.is_empty()
            && self.cleared_default.is_none()
            && self.open_public_gates.is_empty()
    }

    /// The sentence a declassification confirmation shows (design §3.11).
    /// `None` when nothing is affected.
    pub fn confirmation_sentence(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut parts = Vec::new();
        if !self.monitors.is_empty() {
            let names: Vec<String> = self
                .monitors
                .iter()
                .map(|(_, name, _)| format!("\"{name}\""))
                .collect();
            let models = distinct(self.monitors.iter().map(|(_, _, model)| model.clone()));
            let (noun, verb) = if names.len() == 1 {
                ("Monitor", "uses")
            } else {
                ("Monitors", "use")
            };
            parts.push(format!(
                "{noun} {} {verb} {} and will be turned off.",
                join_and(&names),
                join_and(&models)
            ));
        }
        if let Some(model) = &self.cleared_default {
            parts.push(format!(
                "The default decision model {model} is public and will be cleared."
            ));
        }
        for (phase, model) in &self.open_public_gates {
            parts.push(format!(
                "The gate on phase \"{phase}\" uses {model} and will read this chat when the \
                 phase completes."
            ));
        }
        Some(parts.join(" "))
    }
}

fn distinct(items: impl Iterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in items {
        if !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// What [`turn_off_public_names`] would do to `data`, without changing it.
pub fn public_names_off_preview(data: &ExtensionData) -> PublicNamesOff {
    let mut copy = data.clone();
    turn_off_public_names_with(&mut copy, "", &declared_name)
}

/// Remove the two records that let a chat's decision models lower it: the
/// person's basis and a spawn ceiling. Both name the chat they were written
/// for, so neither may travel to another row. What a branch or copy of a chat
/// bound to a public model does (design §3.16): its public names stay on,
/// because such a chat is public whatever it names and needs no basis.
pub fn forget_lowering_records(data: &mut ExtensionData) {
    data.extension_states
        .remove(&format!("{PUBLIC_BASIS_STATE}.{PUBLIC_BASIS_VERSION}"));
    data.extension_states.remove(&format!(
        "{PRIVACY_CEILING_STATE}.{PRIVACY_CEILING_VERSION}"
    ));
}

/// Turn a chat's public decision models off in place: disable every enabled
/// monitor whose model declares Public, clear a public default, remove the
/// basis and the ceiling, and leave a [`DecisionNotice`] saying so. Gates are
/// listed, not changed. A settings record this build cannot read is left alone
/// (it lowers nothing, and an edit must not erase what it cannot see).
///
/// Used by declassification (whose note is [`DECLASSIFIED_NOTICE`]) and by
/// every derived row: a branch, a copy, an import ([`DERIVED_NOTICE`]).
pub fn turn_off_public_names(data: &mut ExtensionData, notice: &str) -> PublicNamesOff {
    turn_off_public_names_with(data, notice, &declared_name)
}

pub fn turn_off_public_names_with(
    data: &mut ExtensionData,
    notice: &str,
    declare: &dyn Fn(&DecisionModelRef) -> Option<DeclaredName>,
) -> PublicNamesOff {
    let is_public = |model: &DecisionModelRef| {
        declare(model).is_some_and(|declared| !declared.tier.is_private())
    };
    let mut off = PublicNamesOff::default();
    let stored = data
        .get_extension_state(DECISION_SETTINGS_STATE, DECISION_SETTINGS_VERSION)
        .filter(|value| !value.is_null())
        .cloned();
    if let Some(value) = stored {
        if let Ok(mut settings) = serde_json::from_value::<DecisionSettings>(value) {
            if settings.default_model.as_ref().is_some_and(&is_public) {
                off.cleared_default = settings.default_model.take().map(|m| m.label());
            }
            for monitor in settings.monitors.iter_mut().filter(|m| m.enabled) {
                if monitor.model.as_ref().is_some_and(&is_public) {
                    monitor.enabled = false;
                    off.monitors.push((
                        monitor.id.clone(),
                        monitor.name.clone(),
                        monitor
                            .model
                            .as_ref()
                            .map(|m| m.label())
                            .unwrap_or_default(),
                    ));
                }
            }
            if !off.monitors.is_empty() || off.cleared_default.is_some() {
                if let Ok(value) = serde_json::to_value(&settings) {
                    data.set_extension_state(
                        DECISION_SETTINGS_STATE,
                        DECISION_SETTINGS_VERSION,
                        value,
                    );
                }
            }
        }
    }
    let plan: Option<OrchestrationPlan> = data
        .get_extension_state(PLAN_STATE, PLAN_VERSION)
        .and_then(|value| serde_json::from_value(value.clone()).ok());
    if let Some(plan) = plan {
        for phase in plan.phases.iter().filter(|p| !p.status.is_closed()) {
            if let Some(model) = phase.gate.as_ref().and_then(|g| g.model.as_ref()) {
                if is_public(model) {
                    off.open_public_gates
                        .push((phase.title.clone(), model.label()));
                }
            }
        }
    }
    forget_lowering_records(data);
    if !notice.is_empty() && (!off.monitors.is_empty() || off.cleared_default.is_some()) {
        let note = DecisionNotice {
            text: notice.to_string(),
            monitors: off.monitors.iter().map(|(id, _, _)| id.clone()).collect(),
            cleared_default: off.cleared_default.clone(),
            at: Utc::now(),
        };
        if let Ok(value) = serde_json::to_value(note) {
            data.set_extension_state(DECISION_NOTICE_STATE, DECISION_NOTICE_VERSION, value);
        }
    }
    off
}

#[cfg(test)]
#[path = "decision_axes_tests.rs"]
mod tests;
