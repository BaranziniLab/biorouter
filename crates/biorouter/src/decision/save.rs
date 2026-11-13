//! Storing a chat's decision settings after checking that what they turn on
//! can run, and what the save does to the chat's privacy tier.
//!
//! [`settings::save`] and [`settings::save_chat`] only validate the record's
//! shape: they are **unchecked** writes, and store a monitor whose model is
//! not there, cannot take its question or may not read the chat. A door a
//! person saves through (the settings routes, the CLI, a workflow's seed)
//! calls [`save_checked_by`] (or [`save_checked`], [`save_chat_checked`])
//! instead, which ask the decision service first
//! ([`DecisionService::check_settings`],
//! [`DecisionService::check_chat_question`]) and send nothing of the chat.
//!
//! # The tier effect (DR-33)
//!
//! A chat is as private as its least private model, its standing decision
//! models included (`privacy::decision_axes`). So a save can change whether a
//! chat is private, and every door computes that first, with [`tier_change`]:
//! from two folds over the stored row and the row as the save would leave it,
//! never from the provider slot, and with the bound LANGUAGE model's tier
//! ([`DecisionSession::bound_model_tier`]). The daemon, the offline CLI and a
//! save after a restart therefore agree whenever they agree on that tier.
//!
//! - `becomesPublic` (a person names the first public decision model on an
//!   empty, or declassified, chat bound to a private model) is a lowering. It
//!   needs the person's confirmation at the door ([`LoweringConsent`]); without
//!   it the save answers [`NeedsPublicDecisionConfirmation`] and stores
//!   nothing, and a door that may not lower at all (a keyless daemon, SD-16)
//!   answers [`PublicDecisionModelsNotAllowed`]. The person's acceptance is
//!   written as `public_basis.v1` in the SAME statement as the settings, by
//!   `SessionManager::write_public_basis`, which checks eligibility again
//!   inside the write; a first message that raced the save makes it
//!   [`PublicBasisNotEligible`].
//! - `staysPublic` and `becomesPrivateAtNextMessage` are admitted for a person
//!   (the second clears the basis in the same statement).
//! - `refusedNotEligible` and `refusedPrivate` are refused at every door with
//!   the sentence for its reader ([`RefusalAudience`]).
//! - An agent's save is refused whenever it would move the tier at all: only
//!   the person can make a chat public, or private again, by its decision
//!   models (design §1.8, §3.14).
//!
//! Errors, by `downcast_ref` on the returned error:
//!
//! - [`InvalidDecisionSettings`]: the record itself is malformed or out of
//!   bounds (a route answers 400);
//! - [`UnusableDecisionSettings`]: it is valid, but what it turns on cannot
//!   run for this chat now, or would change its tier in a way this door may
//!   not (422);
//! - [`NeedsPublicDecisionConfirmation`]: it would make the chat public and
//!   the person has not confirmed that (409, `needsPublicDecisionConfirmation`);
//! - [`PublicBasisNotEligible`]: the chat started while the lowering was being
//!   saved (409);
//! - [`PublicDecisionModelsNotAllowed`]: this door may not make a chat public
//!   by a decision model (403, SD-16).
//!
//! A door that holds no agent for the chat (the CLI with no daemon running)
//! builds the chat it checks with [`load_for_save`], which carries the tier
//! the session row declares for the model it is bound to
//! ([`row_bound_model_tier`]), so a chat still Public but bound to a private
//! model is checked as the daemon checks it.

use std::sync::Arc;

use super::service::{DecisionService, DecisionSession};
use super::settings::{
    self, DecisionChatSettings, DecisionSettings, InvalidDecisionSettings, DECISION_SETTINGS_STATE,
    DECISION_SETTINGS_VERSION,
};
use super::types::DecisionModelRef;
use crate::agents::orchestration_plan::OrchestrationPlan;
use crate::privacy::alt_provider::RefusalAudience;
use crate::privacy::decision_axes::{
    self, BasisAuthor, BasisIntent, HostLabel, LoweredBy, NameSource, PublicBasis, RowRecords,
    TierEffect,
};
use crate::privacy::ProviderTier;
use crate::providers::decision::catalog::{BASETEN_DECISION_PROVIDER, OLLAMA_DECISION_PROVIDER};
use crate::session::extension_data::ExtensionData;
use crate::session::session_manager::{
    public_basis_not_eligible_sentence, BasisWrite, BasisWriteOutcome,
};
use crate::session::{Session, SessionManager};

/// Settings that are valid but turn on something that cannot run for this
/// chat now (a monitor whose model is not there, cannot answer its question,
/// will not load, or may not read the chat), or that would change the chat's
/// privacy tier in a way this door may not. The message is the sentence a
/// person reads, starting "Nothing was saved".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnusableDecisionSettings(pub String);

impl std::fmt::Display for UnusableDecisionSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UnusableDecisionSettings {}

/// The save would make the chat public by a decision model and the person has
/// not confirmed that at this door. Nothing was saved. `sentence` is the
/// effect line ("This chat will be public: joey-1 runs on Baseten.") the
/// client asks the person about; `models` are the public models, as
/// `provider/model`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsPublicDecisionConfirmation {
    pub sentence: String,
    pub models: Vec<String>,
}

impl std::fmt::Display for NeedsPublicDecisionConfirmation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.sentence)
    }
}

impl std::error::Error for NeedsPublicDecisionConfirmation {}

/// The chat started (a message landed) between the check and the write of a
/// lowering, so it can no longer be made public by a decision model. Nothing
/// was saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicBasisNotEligible(pub String);

impl std::fmt::Display for PublicBasisNotEligible {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PublicBasisNotEligible {}

/// This door may not make a chat public by a decision model at all (a keyless
/// daemon its operator did not open to it, SD-16). Nothing was saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicDecisionModelsNotAllowed(pub String);

impl std::fmt::Display for PublicDecisionModelsNotAllowed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PublicDecisionModelsNotAllowed {}

/// Whether the person confirmed, at this door, that a save may make the chat
/// public by a decision model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoweringConsent {
    /// The person saw the effect line and accepted it: the desktop's
    /// `acceptPublicDecisionModels: true` under the user-action proof, or a
    /// confirmation typed at a terminal.
    Confirmed,
    /// Not confirmed: a lowering answers [`NeedsPublicDecisionConfirmation`].
    Unconfirmed,
    /// This door never lowers: a lowering answers
    /// [`PublicDecisionModelsNotAllowed`] with this sentence.
    NotAllowedHere(String),
}

/// Who is saving, and what they may do to the chat's tier.
#[derive(Debug, Clone)]
pub struct SaveDoor<'a> {
    /// Who reads a refusal.
    pub audience: RefusalAudience,
    /// For a person's door; an agent's door never lowers whatever this says.
    pub consent: LoweringConsent,
    /// Who the basis says accepted a lowering.
    pub author: BasisAuthor,
    /// The audit word the basis records (`decision-settings`, `phase-gate`,
    /// `agent-start`, `cli`, `schedule`).
    pub door: &'a str,
    /// The chat's subagents running now, whose copies of an "also watch"
    /// monitor this save would change (design §3.5). Empty where the door
    /// cannot see them (the CLI with no daemon).
    pub live_children: &'a [String],
}

impl<'a> SaveDoor<'a> {
    /// A person's door in the app (a dialog, a route under the proof).
    pub fn person(door: &'a str, consent: LoweringConsent) -> Self {
        Self {
            audience: RefusalAudience::Person,
            consent,
            author: BasisAuthor::Person,
            door,
            live_children: &[],
        }
    }

    /// A person at a terminal.
    pub fn cli(consent: LoweringConsent) -> Self {
        Self {
            audience: RefusalAudience::Cli,
            ..Self::person("cli", consent)
        }
    }

    /// An agent's door (a tool call). It never lowers and never raises.
    pub fn agent(door: &'a str) -> Self {
        Self {
            audience: RefusalAudience::Agent,
            ..Self::person(door, LoweringConsent::Unconfirmed)
        }
    }

    pub fn with_live_children(mut self, live_children: &'a [String]) -> Self {
        self.live_children = live_children;
        self
    }

    pub fn with_author(mut self, author: BasisAuthor) -> Self {
        self.author = author;
        self
    }
}

/// What a save leaves standing, in place of what the row holds.
#[derive(Debug, Clone, Copy)]
pub enum AfterSave<'a> {
    /// The chat's decision settings as the save stores them.
    Settings(&'a DecisionSettings),
    /// The chat's plan as a gate save stores it.
    Plan(Option<&'a OrchestrationPlan>),
    /// Both, as a workflow's seed stores them.
    Both(&'a DecisionSettings, Option<&'a OrchestrationPlan>),
}

/// What a save does to the chat's tier, worked out before it writes
/// ([`tier_change`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierChange {
    pub effect: TierEffect,
    /// The public decision models the save leaves standing, in the fold's
    /// order: the effect line's names and the basis's models.
    pub public_names: Vec<LoweredBy>,
    /// What the write does with `public_basis.v1`, in the same statement.
    pub basis: BasisWrite,
    /// Whether the chat will be public after the save because of its decision
    /// models ([`DecisionSession::lowered_after_save`]).
    pub lowered_after_save: bool,
}

impl TierChange {
    /// The line a door shows for this change ([`tier_effect_line`]).
    pub fn effect_line(&self) -> Option<String> {
        tier_effect_line(self.effect, &self.public_names)
    }
}

/// What [`save_checked_by`] stored, and what it did to the chat's tier.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedSave {
    pub settings: DecisionSettings,
    pub tier_effect: TierEffect,
    /// The line to show beside the answer ([`tier_effect_line`]), or `None`.
    pub effect_line: Option<String>,
}

impl CheckedSave {
    /// Whether this save made the chat public: a daemon door then calls
    /// `Agent::lower_decision_axes` on the chat's agent if it holds one.
    pub fn lowered(&self) -> bool {
        self.tier_effect == TierEffect::BecomesPublic
    }
}

/// The InfoTip beside "This chat will be public" (design §3.2).
pub const BECOMES_PUBLIC_INFO: &str = "A chat is as private as its least private model. Its \
     language model keeps answering, but the chat can't use private extensions, private \
     knowledge bases or private chats. Choose a model on this computer to keep it private.";

/// The one line a door shows for a save's tier effect (design §3.2), or `None`
/// for an effect that says nothing.
pub fn tier_effect_line(effect: TierEffect, names: &[LoweredBy]) -> Option<String> {
    match effect {
        TierEffect::BecomesPublic => Some(format!("This chat will be public: {}.", runs_on(names))),
        // No public name of its own: it is public through its spawn ceiling.
        TierEffect::StaysPublic if names.is_empty() => {
            Some("This chat stays public: it is part of a public chat's work.".to_string())
        }
        TierEffect::StaysPublic => Some(format!("This chat stays public: {}.", runs_on(names))),
        TierEffect::BecomesPrivateAtNextMessage => Some(
            "This chat becomes private at your next message. Public models can't be used in it \
             after that unless you make it public from History."
                .to_string(),
        ),
        TierEffect::None | TierEffect::RefusedNotEligible | TierEffect::RefusedPrivate => None,
    }
}

/// "joey-1 runs on Baseten", "joey-1 and elephant-1 run on Baseten", the
/// models grouped by where they run, each named once.
fn runs_on(names: &[LoweredBy]) -> String {
    let mut places: Vec<(String, Vec<String>)> = Vec::new();
    for name in names {
        let place = name.host.place();
        match places.iter_mut().find(|(p, _)| *p == place) {
            Some((_, models)) if models.contains(&name.model_name()) => {}
            Some((_, models)) => models.push(name.model_name()),
            None => places.push((place, vec![name.model_name()])),
        }
    }
    if places.is_empty() {
        return "a public decision model".to_string();
    }
    places
        .into_iter()
        .map(|(place, models)| {
            let verb = if models.len() == 1 { "runs" } else { "run" };
            format!("{} {verb} on {place}", and_list(&models))
        })
        .collect::<Vec<_>>()
        .join(", and ")
}

fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// `provider/model` for a counted public name.
fn model_label(name: &LoweredBy) -> String {
    let provider = match name.host {
        HostLabel::Baseten => BASETEN_DECISION_PROVIDER,
        HostLabel::ThisComputer | HostLabel::OllamaAt(_) | HostLabel::OllamaCloud => {
            OLLAMA_DECISION_PROVIDER
        }
    };
    format!("{provider}/{}", name.model)
}

/// `provider/model` for each counted public name, each model once, in the
/// order first named (a monitor and a gate on one model are one model).
fn model_labels(names: &[LoweredBy]) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    for label in names.iter().map(model_label) {
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    labels
}

/// What a refusal says could not run.
fn subject(name: &LoweredBy) -> String {
    match name.source {
        NameSource::Default => format!(
            "the chat's default decision model {} could not run",
            model_label(name)
        ),
        NameSource::Monitor => format!("monitor \"{}\" could not run", name.label),
        NameSource::Gate => format!("the gate on phase \"{}\" could not run", name.label),
        NameSource::PhaseMonitor => {
            format!("subagent monitor \"{}\" could not run", name.label)
        }
        NameSource::Parent => "this chat's decision models could not run".to_string(),
    }
}

/// The way out a refusal ends with, by its reader.
fn way_out(name: &LoweredBy, audience: RefusalAudience) -> String {
    match audience {
        RefusalAudience::Person => format!(
            "Start a new chat and choose {} before its first message, or make this chat public \
             from History.",
            name.model_name()
        ),
        RefusalAudience::Cli => format!(
            "Choose a model on this computer (`biorouter models decision list` shows them), or \
             start a new chat with `biorouter session --decision-model {}`.",
            model_label(name)
        ),
        RefusalAudience::Agent => agent_way_out(),
    }
}

/// The way out an agent's refusal ends with: it cannot change the tier, the
/// person can.
fn agent_way_out() -> String {
    format!(
        "This would change whether this chat is private, or needs a model a private chat can't \
         use, which only the person can choose. Name a private decision model {}, or tell the \
         person; do not retry.",
        crate::privacy::alt_provider::MODEL_LIST_ADVICE_AGENT
    )
}

/// What an agent is told when it names `model`, a public decision model, for
/// a private chat at a door that does not go through [`tier_change`]: a
/// spawn's monitors, which the child is born with at the parent's
/// classification (design §3.4). The same reason [`tier_change`] gives an
/// agent for [`TierEffect::RefusedPrivate`] (live QA M3-2), without the
/// subject, so the door names what could not be put where. `None` when the
/// model is not declared public: doubt never refuses here, and the door's own
/// usability check decides.
pub fn agent_public_model_on_private_chat(model: &DecisionModelRef) -> Option<String> {
    let declared = crate::privacy::decision_axes::declared_name(model)?;
    if declared.tier.is_private() {
        return None;
    }
    Some(format!(
        "this chat is private, so {}, {}, can't read it. {}",
        crate::providers::decision::catalog::decision_model_short_name(model),
        declared.host.model_noun(),
        agent_way_out()
    ))
}

/// The reason, without the leading "Nothing was saved", that `effect` refuses
/// a save naming `name`, for `audience`. `None` for an effect a person may
/// save.
fn refusal_reason(
    effect: TierEffect,
    name: &LoweredBy,
    audience: RefusalAudience,
) -> Option<String> {
    let noun = name.host.model_noun();
    let place = name.host.place();
    Some(match effect {
        TierEffect::RefusedPrivate => format!(
            "{}. This chat is private, so {}, {noun}, can't read it. {}",
            subject(name),
            name.model_name(),
            way_out(name, audience)
        ),
        TierEffect::RefusedNotEligible => format!(
            "this chat already has messages on a private model, so it becomes private at its \
             next message and {} · {place} couldn't read it. {}",
            name.model_name(),
            way_out(name, audience)
        ),
        _ => return None,
    })
}

/// What an agent's save is told when it would change whether the chat is
/// private, which only the person can do (design §1.8).
pub const AGENT_MAY_NOT_MOVE_THE_TIER: &str = "This would change whether this chat is private, \
     which only the person can do. Tell the person; do not retry.";

/// The first public name the save adds (in `after`, not in `before`), else
/// the first it leaves standing.
fn named_by_the_save(before: &[LoweredBy], after: &[LoweredBy]) -> Option<LoweredBy> {
    after
        .iter()
        .find(|name| !before.contains(name))
        .or_else(|| after.first())
        .cloned()
}

/// What saving `after` over `row` does to the chat's tier, at `door`, with the
/// chat as `session` holds it (its bound language model's tier and the master
/// switch as the door sampled it). Pure apart from the `OLLAMA_HOST` read the
/// fold makes; nothing is written.
///
/// `Err` is the refusal, by `downcast_ref` (see the module docs):
/// [`UnusableDecisionSettings`], [`NeedsPublicDecisionConfirmation`] or
/// [`PublicDecisionModelsNotAllowed`]. With the master switch off nothing is
/// refused and no basis is set (one the save leaves with no public name is
/// still cleared, so a name saved while tiers were off can never make the chat
/// public when they come back on, design §2.5).
pub fn tier_change(
    session: &DecisionSession,
    row: &Session,
    after: AfterSave<'_>,
    door: &SaveDoor<'_>,
) -> anyhow::Result<TierChange> {
    let records = RowRecords::parse(&row.extension_data);
    let before = records.inputs(row);
    let after_inputs = match after {
        AfterSave::Settings(settings) => before.with_settings(settings),
        AfterSave::Plan(plan) => before.with_plan(plan),
        AfterSave::Both(settings, plan) => before.with_settings(settings).with_plan(plan),
    };
    // No model bound reads as Public, as `Agent::bound_model_tier` does: such
    // a save cannot lower the chat.
    let bound = session.bound_model_tier.unwrap_or(ProviderTier::Public);
    let eligibility = decision_axes::eligibility(row);
    let effect = if session.enforced {
        decision_axes::tier_effect(bound, &before, &after_inputs, eligibility)
    } else {
        TierEffect::None
    };
    let before_public = decision_axes::public_standing_names(&before);
    let public_names = decision_axes::public_standing_names(&after_inputs);
    if let Some(name) = named_by_the_save(&before_public, &public_names) {
        if let Some(reason) = refusal_reason(effect, &name, door.audience) {
            return Err(UnusableDecisionSettings(format!("Nothing was saved: {reason}")).into());
        }
    }
    if door.audience == RefusalAudience::Agent && effect.moves_the_tier() {
        return Err(UnusableDecisionSettings(format!(
            "Nothing was saved. {AGENT_MAY_NOT_MOVE_THE_TIER}"
        ))
        .into());
    }
    if effect == TierEffect::BecomesPublic {
        match &door.consent {
            LoweringConsent::Confirmed => {}
            LoweringConsent::Unconfirmed => {
                return Err(NeedsPublicDecisionConfirmation {
                    sentence: tier_effect_line(effect, &public_names).unwrap_or_default(),
                    models: model_labels(&public_names),
                }
                .into())
            }
            LoweringConsent::NotAllowedHere(sentence) => {
                return Err(PublicDecisionModelsNotAllowed(sentence.clone()).into())
            }
        }
    }
    let intent = decision_axes::basis_intent(effect, bound, eligibility, !public_names.is_empty());
    let basis = match intent {
        // An agent never writes a basis (its `staysPublic` save keeps the one
        // the person wrote), and neither does a save that does not know the
        // model the chat is bound to: a basis written silently for a public
        // model must name a model someone bound.
        BasisIntent::Set
            if door.audience == RefusalAudience::Agent
                || session.bound_model_tier.is_none()
                || !session.enforced =>
        {
            BasisWrite::Keep
        }
        BasisIntent::Set => BasisWrite::Set(PublicBasis::new(
            row.id.clone(),
            door.author,
            door.door,
            model_labels(&public_names),
        )),
        BasisIntent::Clear => BasisWrite::Clear,
        BasisIntent::Keep => BasisWrite::Keep,
    };
    Ok(TierChange {
        effect,
        lowered_after_save: matches!(effect, TierEffect::BecomesPublic | TierEffect::StaysPublic),
        public_names,
        basis,
    })
}

/// The refusal an agent's door owes for saving `after` over `row` (design
/// §1.8, §3.14), or `None` when it may: refused whenever the save would make
/// the chat public, make it private at its next message, or name a public
/// model the chat cannot use. The sentence is the agent's, without a leading
/// "Nothing was saved", so the door can say what did not happen in its own
/// words. Nothing is written, and the door writes no basis.
pub fn agent_tier_refusal(
    session: &DecisionSession,
    row: &Session,
    after: AfterSave<'_>,
) -> Option<String> {
    let error = tier_change(session, row, after, &SaveDoor::agent("agent")).err()?;
    let text = error.to_string();
    Some(
        text.strip_prefix("Nothing was saved: ")
            .or_else(|| text.strip_prefix("Nothing was saved. "))
            .map(str::to_string)
            .unwrap_or(text),
    )
}

/// Validate `settings`, check what they do to the chat's tier
/// ([`tier_change`]) and that what they turn on could run for `session`
/// ([`DecisionService::check_settings`], against the record stored now, where
/// an unreadable one counts as none), then store them with the basis the
/// change asks for, in one statement (`SessionManager::write_public_basis`).
///
/// A daemon door whose answer [`CheckedSave::lowered`] calls
/// `Agent::lower_decision_axes` on the chat's agent if it holds one.
pub async fn save_checked_by(
    service: &DecisionService,
    manager: &SessionManager,
    session: &DecisionSession,
    settings: DecisionSettings,
    door: &SaveDoor<'_>,
) -> anyhow::Result<CheckedSave> {
    settings::validate(&settings).map_err(InvalidDecisionSettings)?;
    let id = session.session_id.as_str();
    let row = manager.get_session(id, false).await?;
    let stored = settings::load(manager, id).await.ok();
    let change = tier_change(session, &row, AfterSave::Settings(&settings), door)?;
    settings::refuse_public_copies_on_private_children(
        manager,
        &settings,
        stored.as_ref(),
        door.live_children,
    )
    .await
    .map_err(UnusableDecisionSettings)?;
    let checked = session
        .clone()
        .with_lowered_after_save(change.lowered_after_save);
    service
        .check_settings(&checked, &settings, stored.as_ref())
        .await
        .map_err(UnusableDecisionSettings)?;
    let value = serde_json::to_value(&settings)?;
    let written = manager
        .write_public_basis(id, change.basis.clone(), move |data: &mut ExtensionData| {
            data.set_extension_state(DECISION_SETTINGS_STATE, DECISION_SETTINGS_VERSION, value);
            Ok(())
        })
        .await?;
    match written {
        BasisWriteOutcome::Written(()) => Ok(CheckedSave {
            settings,
            tier_effect: change.effect,
            effect_line: change.effect_line(),
        }),
        BasisWriteOutcome::NotEligible => {
            let model = change
                .public_names
                .first()
                .map(|name| name.model_name())
                .unwrap_or_else(|| "a public decision model".to_string());
            Err(PublicBasisNotEligible(public_basis_not_eligible_sentence(&model)).into())
        }
        BasisWriteOutcome::SessionNotFound => anyhow::bail!("No chat {id} exists."),
    }
}

/// [`save_checked_by`] at a person's door that has not confirmed a lowering
/// (a save that would make the chat public answers
/// [`NeedsPublicDecisionConfirmation`]). Returns the stored value. Kept for
/// the doors written before DR-33; a door that can ask the person uses
/// [`save_checked_by`] with [`LoweringConsent::Confirmed`] once they agreed.
pub async fn save_checked(
    service: &DecisionService,
    manager: &SessionManager,
    session: &DecisionSession,
    settings: DecisionSettings,
) -> anyhow::Result<DecisionSettings> {
    let door = SaveDoor::person("decision-settings", LoweringConsent::Unconfirmed);
    save_checked_by(service, manager, session, settings, &door)
        .await
        .map(|saved| saved.settings)
}

/// [`save_checked`] for a decision chat's question: validated, checked
/// against the decision model the chat is bound to now (read off its
/// session row) when its question is new or of another kind, then stored.
pub async fn save_chat_checked(
    service: &DecisionService,
    manager: &SessionManager,
    session: &DecisionSession,
    chat: DecisionChatSettings,
) -> anyhow::Result<DecisionChatSettings> {
    settings::validate_chat(&chat).map_err(InvalidDecisionSettings)?;
    let stored = settings::load_chat(manager, &session.session_id)
        .await
        .ok()
        .flatten();
    let row = manager.get_session(&session.session_id, false).await?;
    let model = bound_decision_model(row.provider_name.as_deref(), row.model_config.as_ref());
    service
        .check_chat_question(
            session,
            model.as_ref(),
            &chat.question,
            stored.as_ref().map(|stored| &stored.question),
        )
        .await
        .map_err(UnusableDecisionSettings)?;
    settings::save_chat(manager, &session.session_id, chat).await
}

/// The chat a save checks, built by a door that holds no agent for it (the
/// CLI with no daemon running): [`DecisionSession::load`] with the tier the
/// session row declares for the model it is bound to
/// ([`row_bound_model_tier`]) and the row's tier context
/// ([`DecisionSession::with_row_tier_context`]).
///
/// Without that tier, a chat bound to a private model before its first turn
/// (still classified Public) was checked on its classification alone, and the
/// effect of naming a public decision model could not be told.
pub async fn load_for_save(
    manager: Arc<SessionManager>,
    session_id: &str,
    enforced: bool,
) -> anyhow::Result<DecisionSession> {
    let row = manager.get_session(session_id, false).await?;
    let bound = row_bound_model_tier(&row, enforced).await;
    Ok(DecisionSession::load(manager, session_id, enforced)
        .await?
        .with_bound_model_tier(bound)
        .with_row_tier_context(&row))
}

/// [`row_bound_model_tier`], under the name the doors used before DR-33.
pub async fn row_bound_tier(session: &Session, enforced: bool) -> Option<ProviderTier> {
    row_bound_model_tier(session, enforced).await
}

/// The tier of the language model `session`'s row records it bound to
/// (`provider_name`, `model_config`), for the save-time checks only
/// ([`DecisionSession::bound_model_tier`]), declared without building the
/// provider (that would need its credentials).
///
/// It matters only for a chat still classified Public with privacy tiers on,
/// so `None` otherwise, and `None` for a row that names no provider. The
/// declaration is the provider registry's
/// ([`crate::workflow::privacy::declared_provider_tier`], what the scheduler
/// and the CLI's own capability use), except where the model decides it the
/// way the live instance does (design §3.2):
///
/// - `ollama` on a cloud tag (`gpt-oss:120b-cloud`) is Public, since Ollama
///   forwards it to ollama.com; on any other tag it is the tier of the
///   resolved `OLLAMA_HOST` (loopback is Private, any other host Public).
/// - a decision chat's provider (`ollama_decision`, `baseten_decision`) is the
///   tier its adapter declares (`providers::decision::chat::declared_axes`).
///
/// An `OLLAMA_HOST` that does not resolve, and a provider the registry does
/// not list, fall back to the registry's answer (Public for an unlisted one).
/// The residual the declaration always has: an `ollama` model renamed from a
/// cloud model with `ollama cp` declares Private here while its instance is
/// Public, so such a chat may be asked to confirm a lowering the daemon,
/// holding its agent, would call a public chat's save.
pub async fn row_bound_model_tier(session: &Session, enforced: bool) -> Option<ProviderTier> {
    if !enforced || session.privacy_tier.is_private() {
        return None;
    }
    let provider = session
        .provider_name
        .as_deref()
        .map(str::trim)
        .filter(|provider| !provider.is_empty())?;
    let model = session
        .model_config
        .as_ref()
        .map(|config| config.model_name.trim())
        .unwrap_or_default();
    Some(match provider {
        "ollama" if crate::providers::utils::is_ollama_cloud_model(model) => ProviderTier::Public,
        "ollama" => match crate::providers::decision::catalog::ollama_base_url() {
            Ok(base) => crate::providers::self_hosted_tier(base.as_str()),
            Err(_) => crate::workflow::privacy::declared_provider_tier(provider).await,
        },
        BASETEN_DECISION_PROVIDER => {
            crate::providers::decision::chat::declared_axes(
                crate::providers::decision::chat::DecisionChatKind::Baseten,
                model,
            )
            .0
        }
        OLLAMA_DECISION_PROVIDER => {
            crate::providers::decision::chat::declared_axes(
                crate::providers::decision::chat::DecisionChatKind::Ollama,
                model,
            )
            .0
        }
        _ => crate::workflow::privacy::declared_provider_tier(provider).await,
    })
}

/// The decision model a chat is bound to, when its provider is a decision
/// provider.
fn bound_decision_model(
    provider: Option<&str>,
    model: Option<&crate::model::ModelConfig>,
) -> Option<DecisionModelRef> {
    let provider = provider
        .filter(|provider| crate::providers::decision::chat::is_decision_chat_provider(provider))?;
    let model = model?.model_name.trim();
    (!model.is_empty()).then(|| DecisionModelRef::new(provider, model))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::error::{DecisionError, DecisionResult};
    use crate::decision::model::{DecisionCaps, DecisionModel, DecisionModelEntry};
    use crate::decision::service::DecisionUseGate;
    use crate::decision::types::{
        DecisionAdapter, DecisionKindTag, DecisionRequest, DecisionUsage, ModelVerdicts,
    };
    use crate::decision::DecisionModelSource;
    use crate::privacy::{ProviderTier, SessionClassification};
    use crate::session::session_manager::SessionType;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tempfile::TempDir;

    struct Cloud(Arc<AtomicUsize>);

    #[async_trait]
    impl DecisionModel for Cloud {
        fn model_ref(&self) -> DecisionModelRef {
            DecisionModelRef::new("test_decision", "cloud").with_adapter(DecisionAdapter::SystemOne)
        }
        fn display_name(&self) -> String {
            "cloud".into()
        }
        fn tier(&self) -> ProviderTier {
            ProviderTier::Public
        }
        fn affiliation(&self) -> Option<crate::privacy::affiliation::ModelAffiliation> {
            None
        }
        fn caps(&self) -> DecisionCaps {
            DecisionCaps {
                kinds: vec![DecisionKindTag::Binary],
                max_questions: 64,
                context_tokens: 4_096,
                max_concurrency: 1,
                max_request_bytes: None,
                local: false,
            }
        }
        async fn decide(&self, _: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(ModelVerdicts {
                verdicts: Vec::new(),
                usage: DecisionUsage::default(),
                served_model: None,
            })
        }
    }

    struct OnlyCloud(Arc<AtomicUsize>);

    #[async_trait]
    impl DecisionModelSource for OnlyCloud {
        async fn list(&self) -> Vec<DecisionModelEntry> {
            vec![DecisionModelEntry {
                provider: "test_decision".into(),
                model: "cloud".into(),
                display_name: "cloud".into(),
                context_tokens: 4_096,
                kinds: vec![DecisionKindTag::Binary],
                tier: ProviderTier::Public,
                affiliation: None,
                adapter: DecisionAdapter::SystemOne,
                local: false,
                load_refusal: None,
            }]
        }
        async fn open(&self, wanted: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
            if wanted.model == "cloud" {
                Ok(Arc::new(Cloud(self.0.clone())))
            } else {
                Err(DecisionError::NoModel(format!(
                    "No decision model `{}` is listed.",
                    wanted.label()
                )))
            }
        }
    }

    struct Allow;

    #[async_trait]
    impl DecisionUseGate for Allow {
        async fn check(&self, _: &DecisionSession, _: &dyn DecisionModel) -> DecisionResult<()> {
            Ok(())
        }
    }

    fn settings(monitor_model: &str) -> DecisionSettings {
        serde_json::from_value(serde_json::json!({
            "defaultModel": null,
            "monitors": [{
                "id": "done",
                "name": "Done",
                "question": {"instructions": "Is the task done?", "kind": {"binary": {}}},
                "model": {"provider": "test_decision", "model": monitor_model},
                "threshold": 0.8,
                "option": null,
                "rearmBelow": null
            }]
        }))
        .unwrap()
    }

    /// A checked save stores what can run, refuses what cannot with
    /// [`UnusableDecisionSettings`] and stores nothing, and refuses a malformed
    /// record with [`InvalidDecisionSettings`]. Nothing is sent either way.
    #[tokio::test]
    async fn a_checked_save_stores_only_what_can_run() {
        let dir = TempDir::new().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf());
        let id = manager
            .create_session(dir.path().into(), "chat".into(), SessionType::User)
            .await
            .unwrap()
            .id;
        let calls = Arc::new(AtomicUsize::new(0));
        let service = DecisionService::new(Arc::new(OnlyCloud(calls.clone())))
            .with_use_gate(Arc::new(Allow))
            .with_default_model(None);
        let public = DecisionSession::new(&id, SessionClassification::Public, true);

        let stored = save_checked(&service, &manager, &public, settings("cloud"))
            .await
            .unwrap();
        assert_eq!(settings::load(&manager, &id).await.unwrap(), stored);

        let error = save_checked(&service, &manager, &public, settings("gone"))
            .await
            .unwrap_err();
        let unusable = error
            .downcast_ref::<UnusableDecisionSettings>()
            .expect("an unusable save is typed");
        assert!(
            unusable
                .0
                .starts_with("Nothing was saved: monitor \"Done\" could not run."),
            "{unusable}"
        );
        assert_eq!(
            settings::load(&manager, &id).await.unwrap(),
            stored,
            "nothing was saved"
        );

        let mut malformed = settings("cloud");
        malformed.monitors[0].threshold = 7.0;
        let error = save_checked(&service, &manager, &public, malformed)
            .await
            .unwrap_err();
        assert!(error.downcast_ref::<InvalidDecisionSettings>().is_some());

        // Saving the stored monitor again asks nothing, even where it could
        // not run now; a new chat where it is new is asked.
        let private = DecisionSession::new(&id, SessionClassification::Private, true);
        save_checked(&service, &manager, &private, stored.clone())
            .await
            .unwrap();
        let other = manager
            .create_session(dir.path().into(), "private chat".into(), SessionType::User)
            .await
            .unwrap()
            .id;
        let private = DecisionSession::new(&other, SessionClassification::Private, true);
        let error = save_checked(&service, &manager, &private, settings("cloud"))
            .await
            .unwrap_err();
        let unusable = error
            .downcast_ref::<UnusableDecisionSettings>()
            .expect("an unusable save is typed");
        assert!(unusable.0.contains("This chat is private"), "{unusable}");
        assert_eq!(
            settings::load(&manager, &other).await.unwrap(),
            DecisionSettings::default()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0, "a check sends nothing");
    }

    /// A decision chat's question is checked against the model its session
    /// row binds, and a chat bound to no decision model is not asked.
    #[tokio::test]
    async fn a_decision_chat_question_is_checked_against_the_bound_model() {
        let dir = TempDir::new().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf());
        let calls = Arc::new(AtomicUsize::new(0));
        let service = DecisionService::new(Arc::new(OnlyCloud(calls.clone())))
            .with_use_gate(Arc::new(Allow))
            .with_default_model(None);
        let question: DecisionChatSettings = serde_json::from_value(serde_json::json!({
            "question": {"instructions": "Does the note say MS?", "kind": {"binary": {}}}
        }))
        .unwrap();

        let bound = manager
            .create_session(dir.path().into(), "bound".into(), SessionType::User)
            .await
            .unwrap()
            .id;
        manager
            .update(&bound)
            .provider_name("ollama_decision")
            .model_config(crate::model::ModelConfig::new_or_fail("tev1:0.8b"))
            .apply()
            .await
            .unwrap();
        let session = DecisionSession::new(&bound, SessionClassification::Public, true);
        let error = save_chat_checked(&service, &manager, &session, question.clone())
            .await
            .unwrap_err();
        let unusable = error
            .downcast_ref::<UnusableDecisionSettings>()
            .expect("an unusable save is typed");
        assert!(
            unusable.0.starts_with(
                "Nothing was saved: this decision chat's model ollama_decision/tev1:0.8b could \
                 not answer this question."
            ),
            "{unusable}"
        );
        assert_eq!(settings::load_chat(&manager, &bound).await.unwrap(), None);

        let unbound = manager
            .create_session(dir.path().into(), "unbound".into(), SessionType::User)
            .await
            .unwrap()
            .id;
        let session = DecisionSession::new(&unbound, SessionClassification::Public, true);
        save_chat_checked(&service, &manager, &session, question.clone())
            .await
            .unwrap();
        assert_eq!(
            settings::load_chat(&manager, &unbound).await.unwrap(),
            Some(question)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    /// A door holding no agent for the chat (the CLI with no daemon) reads
    /// the language model it is bound to off its session row
    /// ([`row_bound_model_tier`]): a chat still Public but bound to a private
    /// provider is refused a public decision model the save would not make
    /// standing (a provider the fold does not know, so doubt never lowers),
    /// with the daemon's sentence, and nothing is stored. A row bound to a
    /// public provider, a row bound to none, a private chat and privacy tiers
    /// off carry no private bound tier, and an `ollama` row on a cloud tag
    /// declares Public offline, as its instance does live.
    #[tokio::test]
    async fn a_save_with_no_agent_reads_the_bound_model_off_the_row() {
        let dir = TempDir::new().unwrap();
        let manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let calls = Arc::new(AtomicUsize::new(0));
        let service = DecisionService::new(Arc::new(OnlyCloud(calls.clone())))
            .with_use_gate(Arc::new(Allow))
            .with_default_model(None);

        let bound = bound_chat(
            &manager,
            &dir,
            "bound to a private model",
            Some(("versa_azure", "gpt-5.5")),
        )
        .await;
        let session = load_for_save(manager.clone(), &bound, true).await.unwrap();
        assert_eq!(session.classification, SessionClassification::Public);
        assert_eq!(session.bound_model_tier, Some(ProviderTier::Private));
        let error = save_checked(&service, &manager, &session, settings("cloud"))
            .await
            .unwrap_err();
        let unusable = error
            .downcast_ref::<UnusableDecisionSettings>()
            .expect("an unusable save is typed");
        assert!(
            unusable.0.starts_with(
                "Nothing was saved: monitor \"Done\" could not run. This chat runs on a private \
                 model, so it becomes private at its first message, and cloud is a public \
                 model a private chat cannot use."
            ),
            "{unusable}"
        );
        assert_eq!(
            settings::load(&manager, &bound).await.unwrap(),
            DecisionSettings::default(),
            "nothing was saved"
        );
        let off = load_for_save(manager.clone(), &bound, false).await.unwrap();
        assert_eq!(off.bound_model_tier, None, "privacy tiers off");

        let public = bound_chat(
            &manager,
            &dir,
            "bound to a public model",
            Some(("anthropic", "gpt-5.5")),
        )
        .await;
        let session = load_for_save(manager.clone(), &public, true).await.unwrap();
        assert_eq!(session.bound_model_tier, Some(ProviderTier::Public));
        save_checked(&service, &manager, &session, settings("cloud"))
            .await
            .unwrap();

        let unbound = bound_chat(&manager, &dir, "bound to nothing", None).await;
        let session = load_for_save(manager.clone(), &unbound, true)
            .await
            .unwrap();
        assert_eq!(session.bound_model_tier, None);
        save_checked(&service, &manager, &session, settings("cloud"))
            .await
            .unwrap();

        let cloud_tag = bound_chat(
            &manager,
            &dir,
            "ollama cloud tag",
            Some(("ollama", "gpt-oss:120b-cloud")),
        )
        .await;
        let row = manager.get_session(&cloud_tag, false).await.unwrap();
        assert_eq!(
            row_bound_model_tier(&row, true).await,
            Some(ProviderTier::Public),
            "an Ollama cloud tag runs on ollama.com"
        );

        // A private chat's classification already says it.
        let mut private = manager.get_session(&bound, false).await.unwrap();
        private.privacy_tier = SessionClassification::Private;
        assert_eq!(row_bound_model_tier(&private, true).await, None);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "a check sends nothing");
    }

    // -----------------------------------------------------------------
    // DR-33: what a save does to the chat's tier.
    // -----------------------------------------------------------------

    struct Named {
        model: DecisionModelRef,
        tier: ProviderTier,
    }

    #[async_trait]
    impl DecisionModel for Named {
        fn model_ref(&self) -> DecisionModelRef {
            self.model.clone().with_adapter(DecisionAdapter::SystemOne)
        }
        fn display_name(&self) -> String {
            self.model.model.clone()
        }
        fn tier(&self) -> ProviderTier {
            self.tier
        }
        fn affiliation(&self) -> Option<crate::privacy::affiliation::ModelAffiliation> {
            self.tier
                .is_private()
                .then_some(crate::privacy::affiliation::ModelAffiliation::Local)
        }
        fn caps(&self) -> DecisionCaps {
            DecisionCaps {
                kinds: vec![DecisionKindTag::Binary],
                max_questions: 64,
                context_tokens: 4_096,
                max_concurrency: 1,
                max_request_bytes: None,
                local: self.tier.is_private(),
            }
        }
        async fn decide(&self, _: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
            Err(DecisionError::Invalid("a save never asks".into()))
        }
    }

    /// Baseten's joey-1 (public) and Ollama's tev1:0.8b on this computer.
    struct RealNames;

    const JOEY: (&str, &str) = ("baseten_decision", "joey-1");
    const TEV: (&str, &str) = ("ollama_decision", "tev1:0.8b");

    fn real_tier(model: &str) -> ProviderTier {
        if model == JOEY.1 {
            ProviderTier::Public
        } else {
            ProviderTier::Private
        }
    }

    #[async_trait]
    impl DecisionModelSource for RealNames {
        async fn list(&self) -> Vec<DecisionModelEntry> {
            [JOEY, TEV]
                .into_iter()
                .map(|(provider, model)| DecisionModelEntry {
                    provider: provider.into(),
                    model: model.into(),
                    display_name: model.into(),
                    context_tokens: 4_096,
                    kinds: vec![DecisionKindTag::Binary],
                    tier: real_tier(model),
                    affiliation: real_tier(model)
                        .is_private()
                        .then_some(crate::privacy::affiliation::ModelAffiliation::Local),
                    adapter: DecisionAdapter::SystemOne,
                    local: real_tier(model).is_private(),
                    load_refusal: None,
                })
                .collect()
        }
        async fn open(&self, wanted: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
            Ok(Arc::new(Named {
                model: DecisionModelRef::new(wanted.provider.clone(), wanted.model.clone()),
                tier: real_tier(&wanted.model),
            }))
        }
    }

    async fn bound_chat(
        manager: &SessionManager,
        dir: &TempDir,
        name: &str,
        bound: Option<(&str, &str)>,
    ) -> String {
        let id = manager
            .create_session(dir.path().into(), name.into(), SessionType::User)
            .await
            .unwrap()
            .id;
        if let Some((provider, model)) = bound {
            manager
                .update(&id)
                .provider_name(provider)
                .model_config(crate::model::ModelConfig::new_or_fail(model))
                .apply()
                .await
                .unwrap();
        }
        id
    }

    fn monitor_on(id: &str, (provider, model): (&str, &str)) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "name": id,
            "question": {"instructions": "Is the task done?", "kind": {"binary": {}}},
            "model": {"provider": provider, "model": model},
            "threshold": 0.8,
            "option": null,
            "rearmBelow": null
        })
    }

    fn monitors(monitors: Vec<serde_json::Value>) -> DecisionSettings {
        serde_json::from_value(serde_json::json!({"defaultModel": null, "monitors": monitors}))
            .unwrap()
    }

    fn real_service() -> DecisionService {
        DecisionService::new(Arc::new(RealNames))
            .with_use_gate(Arc::new(Allow))
            .with_default_model(None)
    }

    fn has_basis(row: &Session) -> bool {
        row.extension_data
            .get_extension_state(
                decision_axes::PUBLIC_BASIS_STATE,
                decision_axes::PUBLIC_BASIS_VERSION,
            )
            .is_some()
    }

    fn confirmed() -> SaveDoor<'static> {
        SaveDoor::person("decision-settings", LoweringConsent::Confirmed)
    }

    /// Design §3.2's table, row by row, through the save itself: a person's
    /// first public decision model on an empty Versa chat asks to be
    /// confirmed and stores nothing until it is; confirmed, it makes the chat
    /// public, with the basis written in the same statement; a second public
    /// model keeps it public; removing the last one makes it private at the
    /// next message and clears the basis; a local model moves nothing.
    #[tokio::test]
    async fn a_named_public_decision_model_makes_an_empty_chat_on_a_private_model_public_once_confirmed(
    ) {
        let dir = TempDir::new().unwrap();
        let manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let service = real_service();
        let id = bound_chat(&manager, &dir, "versa", Some(("versa_azure", "gpt-5.5"))).await;
        let session = load_for_save(manager.clone(), &id, true).await.unwrap();

        let joey = monitors(vec![monitor_on("done", JOEY)]);
        let error = save_checked(&service, &manager, &session, joey.clone())
            .await
            .unwrap_err();
        let needs = error
            .downcast_ref::<NeedsPublicDecisionConfirmation>()
            .expect("a lowering without the person's confirmation is a 409");
        assert_eq!(
            needs.sentence,
            "This chat will be public: joey-1 runs on Baseten."
        );
        assert_eq!(needs.models, vec!["baseten_decision/joey-1".to_string()]);
        let row = manager.get_session(&id, false).await.unwrap();
        assert!(!has_basis(&row));
        assert_eq!(
            settings::load(&manager, &id).await.unwrap(),
            DecisionSettings::default()
        );

        let refused = save_checked_by(
            &service,
            &manager,
            &session,
            joey.clone(),
            &SaveDoor::person(
                "decision-settings",
                LoweringConsent::NotAllowedHere("SD-16".into()),
            ),
        )
        .await
        .unwrap_err();
        assert_eq!(
            refused.downcast_ref::<PublicDecisionModelsNotAllowed>(),
            Some(&PublicDecisionModelsNotAllowed("SD-16".into()))
        );

        let saved = save_checked_by(&service, &manager, &session, joey.clone(), &confirmed())
            .await
            .unwrap();
        assert_eq!(saved.tier_effect, TierEffect::BecomesPublic);
        assert!(saved.lowered());
        assert_eq!(
            saved.effect_line.as_deref(),
            Some("This chat will be public: joey-1 runs on Baseten.")
        );
        let row = manager.get_session(&id, false).await.unwrap();
        assert!(has_basis(&row), "the basis lands with the settings");
        assert!(decision_axes::from_row(&row).lowers(ProviderTier::Private));

        let session = load_for_save(manager.clone(), &id, true).await.unwrap();
        let two = monitors(vec![monitor_on("done", JOEY), monitor_on("drift", JOEY)]);
        let saved = save_checked(&service, &manager, &session, two)
            .await
            .unwrap();
        assert_eq!(
            settings::load(&manager, &id).await.unwrap().monitors.len(),
            2,
            "staysPublic needs no confirmation"
        );
        let _ = saved;

        let session = load_for_save(manager.clone(), &id, true).await.unwrap();
        let local = monitors(vec![monitor_on("done", TEV)]);
        let saved = save_checked_by(&service, &manager, &session, local, &confirmed())
            .await
            .unwrap();
        assert_eq!(saved.tier_effect, TierEffect::BecomesPrivateAtNextMessage);
        assert_eq!(
            saved.effect_line.as_deref(),
            Some(
                "This chat becomes private at your next message. Public models can't be used in \
                 it after that unless you make it public from History."
            )
        );
        let row = manager.get_session(&id, false).await.unwrap();
        assert!(
            !has_basis(&row),
            "a raise clears the basis in the same statement"
        );

        // A local model on a fresh Versa chat moves nothing.
        let other = bound_chat(
            &manager,
            &dir,
            "versa local",
            Some(("versa_azure", "gpt-5.5")),
        )
        .await;
        let session = load_for_save(manager.clone(), &other, true).await.unwrap();
        let saved = save_checked_by(
            &service,
            &manager,
            &session,
            monitors(vec![monitor_on("done", TEV)]),
            &confirmed(),
        )
        .await
        .unwrap();
        assert_eq!(saved.tier_effect, TierEffect::None);
        assert!(!has_basis(
            &manager.get_session(&other, false).await.unwrap()
        ));
    }

    /// A chat that has started on a private model, and one already private,
    /// are refused a public decision model with the sentence for its reader
    /// (TRIAGE R4); an agent's door is refused any save that moves the tier.
    #[tokio::test]
    async fn a_started_or_private_chat_is_refused_and_an_agent_never_moves_the_tier() {
        let dir = TempDir::new().unwrap();
        let manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let service = real_service();
        let joey = monitors(vec![monitor_on("done", JOEY)]);

        let started = bound_chat(&manager, &dir, "started", Some(("versa_azure", "gpt-5.5"))).await;
        manager
            .add_message(
                &started,
                &crate::conversation::message::Message::user().with_text("patient notes"),
            )
            .await
            .unwrap();
        let session = load_for_save(manager.clone(), &started, true)
            .await
            .unwrap();
        let error = save_checked_by(&service, &manager, &session, joey.clone(), &confirmed())
            .await
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<UnusableDecisionSettings>().unwrap().0,
            "Nothing was saved: this chat already has messages on a private model, so it \
             becomes private at its next message and joey-1 · Baseten couldn't read it. Start \
             a new chat and choose joey-1 before its first message, or make this chat public \
             from History."
        );
        let cli = save_checked_by(
            &service,
            &manager,
            &session,
            joey.clone(),
            &SaveDoor::cli(LoweringConsent::Confirmed),
        )
        .await
        .unwrap_err();
        assert!(
            cli.to_string().ends_with(
                "Choose a model on this computer (`biorouter models decision list` shows them), \
                 or start a new chat with `biorouter session --decision-model \
                 baseten_decision/joey-1`."
            ),
            "{cli}"
        );

        let private = bound_chat(&manager, &dir, "private", Some(("versa_azure", "gpt-5.5"))).await;
        manager
            .update(&private)
            .raise_privacy(SessionClassification::Private, "turn:versa_azure")
            .apply()
            .await
            .unwrap();
        let session = load_for_save(manager.clone(), &private, true)
            .await
            .unwrap();
        let error = save_checked_by(&service, &manager, &session, joey.clone(), &confirmed())
            .await
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<UnusableDecisionSettings>().unwrap().0,
            "Nothing was saved: monitor \"done\" could not run. This chat is private, so joey-1, \
             a Baseten model, can't read it. Start a new chat and choose joey-1 before its first \
             message, or make this chat public from History."
        );

        let fresh = bound_chat(&manager, &dir, "agent", Some(("versa_azure", "gpt-5.5"))).await;
        let session = load_for_save(manager.clone(), &fresh, true).await.unwrap();
        let row = manager.get_session(&fresh, false).await.unwrap();
        let refusal = agent_tier_refusal(&session, &row, AfterSave::Settings(&joey))
            .expect("an agent never makes a chat public");
        assert_eq!(refusal, AGENT_MAY_NOT_MOVE_THE_TIER);
        assert_eq!(
            agent_tier_refusal(
                &session,
                &row,
                AfterSave::Settings(&monitors(vec![monitor_on("done", TEV)]))
            ),
            None,
            "a local model moves nothing"
        );
        for id in [&started, &private, &fresh] {
            assert_eq!(
                settings::load(&manager, id).await.unwrap(),
                DecisionSettings::default()
            );
        }
    }

    /// A chat on a public model is public whatever it names: no line, and the
    /// basis is written silently when it is eligible, so a later switch to a
    /// private model keeps it public. A save that does not know the bound
    /// model writes none. With the master switch off nothing is refused and
    /// no basis is written, but one the save leaves with no public name is
    /// still cleared.
    #[tokio::test]
    async fn a_public_model_writes_the_basis_silently_and_the_switch_off_writes_none() {
        let dir = TempDir::new().unwrap();
        let manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let service = real_service();
        let joey = monitors(vec![monitor_on("done", JOEY)]);

        let claude = bound_chat(&manager, &dir, "claude", Some(("anthropic", "claude"))).await;
        let session = load_for_save(manager.clone(), &claude, true).await.unwrap();
        let saved = save_checked(&service, &manager, &session, joey.clone())
            .await
            .unwrap();
        let _ = saved;
        assert!(has_basis(
            &manager.get_session(&claude, false).await.unwrap()
        ));

        let unbound = bound_chat(&manager, &dir, "unbound", None).await;
        let session = load_for_save(manager.clone(), &unbound, true)
            .await
            .unwrap();
        save_checked(&service, &manager, &session, joey.clone())
            .await
            .unwrap();
        assert!(
            !has_basis(&manager.get_session(&unbound, false).await.unwrap()),
            "no basis for a model nobody bound"
        );

        let off = bound_chat(&manager, &dir, "off", Some(("versa_azure", "gpt-5.5"))).await;
        let session = load_for_save(manager.clone(), &off, false).await.unwrap();
        let row = manager.get_session(&off, false).await.unwrap();
        let change = tier_change(&session, &row, AfterSave::Settings(&joey), &confirmed()).unwrap();
        assert_eq!(change.effect, TierEffect::None);
        assert_eq!(change.basis, BasisWrite::Keep);
        let change = tier_change(
            &session,
            &row,
            AfterSave::Settings(&DecisionSettings::default()),
            &confirmed(),
        )
        .unwrap();
        assert_eq!(change.basis, BasisWrite::Clear);
    }

    /// A subagent that is public only through its spawn ceiling, and has
    /// messages, can be edited by the person: its saves are `staysPublic`
    /// (the ceiling keeps it public before and after), no basis can be
    /// written there, and none is asked for. Asking for one refused a local
    /// monitor with "this chat has started" (review, 2026-11-11).
    #[tokio::test]
    async fn a_ceilinged_subagent_with_messages_takes_a_person_s_save() {
        let dir = TempDir::new().unwrap();
        let manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let service = real_service();
        let parent = bound_chat(&manager, &dir, "parent", Some(("versa_azure", "gpt-5.5"))).await;
        let child = manager
            .create_session(dir.path().into(), "child".into(), SessionType::SubAgent)
            .await
            .unwrap()
            .id;
        let mut data = manager
            .get_session(&child, false)
            .await
            .unwrap()
            .extension_data;
        decision_axes::PrivacyCeiling::public_from(parent.clone(), None)
            .stamp(&mut data)
            .unwrap();
        manager
            .update(&child)
            .parent_session_id(Some(parent.clone()))
            .provider_name("versa_azure")
            .model_config(crate::model::ModelConfig::new_or_fail("gpt-5.5"))
            .extension_data(data)
            .apply()
            .await
            .unwrap();
        manager
            .add_message(
                &child,
                &crate::conversation::message::Message::user().with_text("fetch it"),
            )
            .await
            .unwrap();
        let row = manager.get_session(&child, false).await.unwrap();
        assert!(
            decision_axes::from_row(&row).lowers(ProviderTier::Private),
            "the fixture must be public through its ceiling"
        );

        for settings in [
            monitors(vec![monitor_on("done", TEV)]),
            monitors(vec![monitor_on("done", JOEY)]),
        ] {
            let session = load_for_save(manager.clone(), &child, true).await.unwrap();
            let saved =
                save_checked_by(&service, &manager, &session, settings.clone(), &confirmed())
                    .await
                    .expect("a person's save on a ceilinged child lands");
            assert_eq!(saved.tier_effect, TierEffect::StaysPublic);
            assert_eq!(settings::load(&manager, &child).await.unwrap(), settings);
            assert!(!has_basis(
                &manager.get_session(&child, false).await.unwrap()
            ));
        }
        let session = load_for_save(manager.clone(), &child, true).await.unwrap();
        let saved = save_checked_by(
            &service,
            &manager,
            &session,
            monitors(vec![monitor_on("done", TEV)]),
            &confirmed(),
        )
        .await
        .unwrap();
        assert_eq!(
            saved.effect_line.as_deref(),
            Some("This chat stays public: it is part of a public chat's work.")
        );
    }

    /// The daemon (bound tier from the agent) and the offline CLI (bound tier
    /// from the row) give the same effect for the same row and edit.
    #[tokio::test]
    async fn the_daemon_and_the_offline_cli_agree_on_the_effect() {
        let dir = TempDir::new().unwrap();
        let manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let id = bound_chat(&manager, &dir, "versa", Some(("versa_azure", "gpt-5.5"))).await;
        let row = manager.get_session(&id, false).await.unwrap();
        let offline = load_for_save(manager.clone(), &id, true).await.unwrap();
        let live = DecisionSession::load(manager.clone(), &id, true)
            .await
            .unwrap()
            .with_bound_model_tier(Some(ProviderTier::Private))
            .with_row_tier_context(&row);
        let joey = monitors(vec![monitor_on("done", JOEY)]);
        let effect = |session: &DecisionSession| {
            tier_change(session, &row, AfterSave::Settings(&joey), &confirmed())
                .unwrap()
                .effect
        };
        assert_eq!(effect(&offline), TierEffect::BecomesPublic);
        assert_eq!(effect(&offline), effect(&live));
        assert_eq!(offline.tier_context, live.tier_context);
    }

    /// Live QA M3-5: the models a confirmation and a basis carry name each
    /// model once, however many sources name it.
    #[test]
    fn model_labels_name_each_model_once() {
        use crate::privacy::decision_axes::{HostLabel, NameSource};
        let name = |source, label: &str, model: &str| LoweredBy {
            source,
            label: label.to_string(),
            model: model.to_string(),
            host: HostLabel::Baseten,
            others: 0,
        };
        let names = [
            name(NameSource::Monitor, "Done", "joey-1"),
            name(NameSource::Gate, "Draft", "joey-1"),
            name(NameSource::Default, "", "elephant-1"),
        ];
        assert_eq!(
            model_labels(&names),
            vec![
                format!("{BASETEN_DECISION_PROVIDER}/joey-1"),
                format!("{BASETEN_DECISION_PROVIDER}/elephant-1"),
            ]
        );
    }

    /// A Baseten model stored by its slug is named by its alias in every
    /// sentence, while the record and the CLI's model label keep the slug
    /// (live check 2026-11-11: the live Monitors dialog read "This chat stays
    /// public: decision-models/baseten-joey-2b-latest runs on Baseten.").
    #[test]
    fn sentences_name_a_baseten_slug_by_its_alias() {
        use crate::privacy::decision_axes::{HostLabel, NameSource};
        let slug = "decision-models/baseten-joey-2b-latest";
        let name = LoweredBy {
            source: NameSource::Monitor,
            label: "Done".to_string(),
            model: slug.to_string(),
            host: HostLabel::Baseten,
            others: 0,
        };
        assert_eq!(name.model_name(), "joey-1");
        let names = [name.clone(), name.clone()];
        assert_eq!(
            tier_effect_line(TierEffect::StaysPublic, &names).as_deref(),
            Some("This chat stays public: joey-1 runs on Baseten.")
        );
        assert_eq!(
            tier_effect_line(TierEffect::BecomesPublic, &names).as_deref(),
            Some("This chat will be public: joey-1 runs on Baseten.")
        );
        assert_eq!(
            name.person_phrase(),
            "monitor \"Done\" uses joey-1, a Baseten model"
        );
        assert_eq!(name.agent_phrase(), "monitor \"Done\", joey-1 on Baseten");
        let refused =
            refusal_reason(TierEffect::RefusedPrivate, &name, RefusalAudience::Person).unwrap();
        assert!(!refused.contains(slug), "{refused}");
        assert!(refused.contains("joey-1"), "{refused}");
        assert_eq!(
            model_labels(&names),
            vec![format!("{BASETEN_DECISION_PROVIDER}/{slug}")]
        );
        let agent = agent_public_model_on_private_chat(&DecisionModelRef::new(
            BASETEN_DECISION_PROVIDER,
            slug,
        ))
        .unwrap();
        assert!(
            agent.starts_with("this chat is private, so joey-1, a Baseten model, can't read it."),
            "{agent}"
        );
        assert_eq!(
            crate::decision::monitor::model_phrase(&DecisionModelRef::new(
                BASETEN_DECISION_PROVIDER,
                slug
            )),
            "joey-1 · Baseten"
        );
    }

    #[test]
    fn only_a_decision_provider_binds_a_decision_chat() {
        let config = crate::model::ModelConfig::new_or_fail("tev1:0.8b");
        assert_eq!(
            bound_decision_model(Some("ollama_decision"), Some(&config)),
            Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b"))
        );
        assert_eq!(bound_decision_model(Some("openai"), Some(&config)), None);
        assert_eq!(bound_decision_model(None, Some(&config)), None);
        assert_eq!(bound_decision_model(Some("ollama_decision"), None), None);
    }
}
