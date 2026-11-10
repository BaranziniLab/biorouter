//! A decision model bound as a chat's main model.
//!
//! A person can pick a decision model (`baseten_decision/joey-1`,
//! `ollama_decision/tev1:0.8b`) in the model picker like any chat model. Such a
//! chat has no LLM loop: each message the person sends is the state, the chat's
//! question (`decision_chat.v1`, or a question line the message starts with) is
//! asked of it, and the reply is one assistant message whose text reads
//! everywhere ("Yes · P(yes) 0.93 · tev1:0.8b") and whose
//! `MessageMetadata::decision_id` names the logged decision, from which the
//! desktop draws a decision card.
//!
//! [`DecisionChatProvider`] is what the factory registers for the two provider
//! ids, so binding goes through Gate A like any model. It declares its tier and
//! affiliation from the model it names (Baseten: Public; Ollama: the
//! self-hosted rule, Public for a cloud tag), never inheriting the default. It
//! takes no tools and refuses to complete chat text: `Agent::reply` answers a
//! decision chat itself (see [`run_decision_chat`]), and every decision still
//! passes Gate H inside the decision service against the chat's classification,
//! however the provider described itself.

use std::marker::PhantomData;

use anyhow::Result;
use async_trait::async_trait;
use indexmap::IndexMap;
use rmcp::model::Tool;

use super::catalog::{
    baseten_entries, decision_model_short_name, discover_ollama, ollama_base_url,
    BASETEN_DECISION_CONTEXT, BASETEN_DECISION_MODELS, BASETEN_DECISION_PROVIDER,
    OLLAMA_DECISION_PROVIDER, OLLAMA_DEFAULT_DECISION_CONTEXT,
};
use super::system_one::{
    BASETEN_API_KEY, BASETEN_DECISION_HOST_KEY, DEFAULT_BASETEN_DECISION_HOST,
};
use crate::conversation::message::Message;
use crate::conversation::Conversation;
use crate::decision::log::{decision_pass_probability, readable_answer};
use crate::decision::service::{DecisionService, DecisionSession};
use crate::decision::settings::DecisionChatSettings;
use crate::decision::types::{
    Answer, Decision, DecisionContext, DecisionKind, DecisionModelRef, DecisionOrigin,
    DecisionRequest, Question, YES,
};
use crate::model::ModelConfig;
use crate::privacy::affiliation::ModelAffiliation;
use crate::privacy::ProviderTier;
use crate::providers::base::{ConfigKey, ModelInfo, Provider, ProviderMetadata, ProviderUsage};
use crate::providers::errors::ProviderError;
use crate::providers::ollama::OLLAMA_HOST;

/// The provider ids a decision model is bound under.
pub const DECISION_CHAT_PROVIDERS: [&str; 2] =
    [BASETEN_DECISION_PROVIDER, OLLAMA_DECISION_PROVIDER];

/// The default Ollama decision model: the one measured on this project's
/// hardware (about 22 ms a question, warm).
pub const OLLAMA_DEFAULT_DECISION_MODEL: &str = "tev1:0.8b";

/// The name the question is sent and logged under.
pub const CHAT_QUESTION_NAME: &str = "question";

/// Whether a bound provider is a decision model.
pub fn is_decision_chat_provider(name: &str) -> bool {
    DECISION_CHAT_PROVIDERS.contains(&name)
}

/// Whether a decision provider has no meter. Ollama bills nothing per token,
/// so an unpriced request there costs nothing; an unpriced Baseten request was
/// billed at a rate Biorouter does not know.
pub fn is_unmetered_decision_provider(name: &str) -> bool {
    name == OLLAMA_DECISION_PROVIDER
}

/// Which decision backend a [`DecisionChatProvider`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionChatKind {
    Baseten,
    Ollama,
}

/// The type-level half of a [`DecisionChatProvider`]: which backend, so each
/// registers under its own id with its own metadata.
pub trait DecisionChatFlavor: Send + Sync + 'static {
    const KIND: DecisionChatKind;
}

#[derive(Debug)]
pub struct BasetenFlavor;
impl DecisionChatFlavor for BasetenFlavor {
    const KIND: DecisionChatKind = DecisionChatKind::Baseten;
}

#[derive(Debug)]
pub struct OllamaFlavor;
impl DecisionChatFlavor for OllamaFlavor {
    const KIND: DecisionChatKind = DecisionChatKind::Ollama;
}

pub type BasetenDecisionChatProvider = DecisionChatProvider<BasetenFlavor>;
pub type OllamaDecisionChatProvider = DecisionChatProvider<OllamaFlavor>;

/// A decision model as a chat's main model.
pub struct DecisionChatProvider<F: DecisionChatFlavor> {
    model: ModelConfig,
    tier: ProviderTier,
    affiliation: Option<ModelAffiliation>,
    _flavor: PhantomData<F>,
}

impl<F: DecisionChatFlavor> std::fmt::Debug for DecisionChatProvider<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecisionChatProvider")
            .field("provider", &provider_id(F::KIND))
            .field("model", &self.model.model_name)
            .field("tier", &self.tier)
            .finish()
    }
}

fn provider_id(kind: DecisionChatKind) -> &'static str {
    match kind {
        DecisionChatKind::Baseten => BASETEN_DECISION_PROVIDER,
        DecisionChatKind::Ollama => OLLAMA_DECISION_PROVIDER,
    }
}

/// The tier and affiliation a decision chat on `model` declares. Baseten is
/// Public with no affiliation, as its chat provider is. Ollama follows the
/// self-hosted rule on the resolved `OLLAMA_HOST`, and a cloud tag is Public
/// whatever the host. A model renamed from a cloud model with `ollama cp`
/// (`remote_host` in `/api/show`) is caught when it is opened for a decision:
/// the adapter's tier then reads Public and Gate H refuses a private chat.
pub fn declared_axes(
    kind: DecisionChatKind,
    model: &str,
) -> (ProviderTier, Option<ModelAffiliation>) {
    match kind {
        DecisionChatKind::Baseten => (ProviderTier::Public, None),
        DecisionChatKind::Ollama => {
            if crate::providers::utils::is_ollama_cloud_model(model) {
                return (ProviderTier::Public, None);
            }
            match ollama_base_url() {
                Ok(base) => (
                    crate::providers::self_hosted_tier(base.as_str()),
                    crate::providers::self_hosted_affiliation(base.as_str()),
                ),
                // An address that does not resolve claims nothing.
                Err(_) => (ProviderTier::Public, None),
            }
        }
    }
}

impl<F: DecisionChatFlavor> DecisionChatProvider<F> {
    /// The provider for `model`, with the window the decision listing gives
    /// it ([`listed_window`]): the window an Ollama model was discovered with
    /// (tev1:0.8b 2,048, clef-flash:latest 16,384), a Baseten model's 16,000.
    /// A window the config states is kept.
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let model = if model.context_limit.is_some() {
            model
        } else {
            let listed = listed_window(F::KIND, &model.model_name).await;
            model.with_context_limit(listed)
        };
        Ok(Self::with_model(model))
    }

    /// The provider for `model` without asking any server. A window the config
    /// states is kept; else the model's registry row, else the backend's
    /// default ([`BASETEN_DECISION_CONTEXT`], [`OLLAMA_DEFAULT_DECISION_CONTEXT`]).
    pub fn with_model(model: ModelConfig) -> Self {
        let (tier, affiliation) = declared_axes(F::KIND, &model.model_name);
        let context = if ModelConfig::has_declared_context_window(&model.model_name) {
            ModelConfig::context_window_for(&model.model_name)
        } else {
            match F::KIND {
                DecisionChatKind::Baseten => BASETEN_DECISION_CONTEXT,
                DecisionChatKind::Ollama => OLLAMA_DEFAULT_DECISION_CONTEXT,
            }
        };
        let model = if model.context_limit.is_some() {
            model
        } else {
            model.with_context_limit(Some(context))
        };
        Self {
            model,
            tier,
            affiliation,
            _flavor: PhantomData,
        }
    }

    /// The decision model this provider names.
    pub fn model_ref(&self) -> DecisionModelRef {
        DecisionModelRef::new(provider_id(F::KIND), &self.model.model_name)
    }
}

/// The window the decision listing gives `model`: Baseten's entry for it by
/// alias or slug, or the window Ollama's discovery found for it
/// (`/api/show`'s `num_ctx`, else the loaded window, else the capped default).
/// `None` when nothing lists it, or the server does not answer.
pub async fn listed_window(kind: DecisionChatKind, model: &str) -> Option<usize> {
    let wanted = DecisionModelRef::new(provider_id(kind), model);
    match kind {
        DecisionChatKind::Baseten => baseten_entries()
            .into_iter()
            .find(|entry| entry.answers_to(&wanted))
            .map(|entry| entry.context_tokens),
        DecisionChatKind::Ollama => {
            let base = ollama_base_url().ok()?;
            discover_ollama(&base)
                .await
                .into_iter()
                .find(|entry| entry.answers_to(&wanted))
                .map(|entry| entry.context_tokens)
        }
    }
}

/// The decision models the Ollama server at `base` lists, by the name each is
/// bound under: exactly the models the decision service discovers there
/// ([`discover_ollama`]), so the chat's model picker, `decision_models` and
/// the Monitors dialog list the same ones.
pub async fn ollama_decision_chat_models(base: &url::Url) -> Vec<String> {
    discover_ollama(base)
        .await
        .into_iter()
        .map(|entry| entry.model)
        .collect()
}

fn metadata_for(kind: DecisionChatKind) -> ProviderMetadata {
    match kind {
        DecisionChatKind::Baseten => {
            let models = BASETEN_DECISION_MODELS
                .iter()
                .map(|known| ModelInfo::new(known.name, known.context_tokens))
                .collect();
            ProviderMetadata::with_models(
                BASETEN_DECISION_PROVIDER,
                "Baseten decision models",
                "Fast models that answer a yes/no, choice or score question about the text \
                 you send, with probabilities, instead of writing text (System One).",
                BASETEN_DECISION_MODELS[0].name,
                models,
                "https://docs.baseten.co",
                vec![
                    ConfigKey::new(BASETEN_API_KEY, true, true, None),
                    ConfigKey::new(
                        BASETEN_DECISION_HOST_KEY,
                        false,
                        false,
                        Some(DEFAULT_BASETEN_DECISION_HOST),
                    ),
                ],
            )
            .with_unlisted_models()
            .with_tier(ProviderTier::Public)
        }
        // No declared list: which decision models exist is what this Ollama
        // has pulled, so every picker asks `fetch_supported_models`, as for
        // the Ollama chat provider. A declared list was read instead of it
        // (the desktop's picker uses a non-empty `known_models` and asks
        // nothing), which listed tev1:0.8b alone, at a window it does not
        // have, while clef-flash:latest was installed (live QA 2026-11-10).
        DecisionChatKind::Ollama => ProviderMetadata::with_models(
            OLLAMA_DECISION_PROVIDER,
            "Ollama decision models",
            "Decision models pulled into a local Ollama (System One): a typed answer with \
             probabilities, computed on this computer.",
            OLLAMA_DEFAULT_DECISION_MODEL,
            Vec::<ModelInfo>::new(),
            "https://ollama.com/search",
            vec![ConfigKey::new(
                "OLLAMA_HOST",
                true,
                false,
                Some(OLLAMA_HOST),
            )],
        )
        .with_unlisted_models()
        // Ollama ships pointed at this machine; an instance pointed off it,
        // or on a cloud tag, says Public itself (`declared_axes`).
        .with_tier(ProviderTier::Private)
        .with_local_compute(),
    }
}

const NOT_A_CHAT_MODEL: &str = "A decision model answers a question about the text you send \
     with probabilities; it does not write chat replies. Pick a chat model for that.";

#[async_trait]
impl<F: DecisionChatFlavor> Provider for DecisionChatProvider<F> {
    fn metadata() -> ProviderMetadata {
        metadata_for(F::KIND)
    }

    fn get_name(&self) -> &str {
        provider_id(F::KIND)
    }

    fn tier(&self) -> ProviderTier {
        self.tier
    }

    fn affiliation(&self) -> Option<ModelAffiliation> {
        self.affiliation
    }

    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        Err(ProviderError::NotImplemented(NOT_A_CHAT_MODEL.to_string()))
    }

    fn get_model_config(&self) -> ModelConfig {
        self.model.clone()
    }

    fn supports_tool_calls(&self) -> bool {
        false
    }

    async fn fetch_supported_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        let names = match F::KIND {
            DecisionChatKind::Baseten => baseten_entries()
                .into_iter()
                .map(|entry| entry.display_name)
                .collect::<Vec<_>>(),
            DecisionChatKind::Ollama => match ollama_base_url() {
                Ok(base) => ollama_decision_chat_models(&base).await,
                Err(_) => Vec::new(),
            },
        };
        Ok(Some(names))
    }

    /// Every decision model listed, unfiltered: the canonical registry knows
    /// none of them, and a filter that matched some would hide the rest.
    async fn fetch_recommended_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        self.fetch_supported_models().await
    }

    /// Named from the first message without a model call: a decision model
    /// cannot write a title.
    async fn generate_session_name(
        &self,
        messages: &Conversation,
    ) -> Result<String, ProviderError> {
        let first = self
            .get_initial_user_messages(messages)
            .into_iter()
            .next()
            .unwrap_or_default();
        Ok(title_from(&first))
    }
}

/// A short title from a message: its question line when it has one, else its
/// first line, cut at a word near 60 characters.
pub fn title_from(text: &str) -> String {
    let line = match parse_question_line(text) {
        Some((question, _)) => question.instructions,
        None => text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("Decision")
            .to_string(),
    };
    let mut title = String::new();
    for word in line.split_whitespace() {
        if title.chars().count() + word.chars().count() + 1 > 60 {
            break;
        }
        if !title.is_empty() {
            title.push(' ');
        }
        title.push_str(word);
    }
    if title.is_empty() {
        line.chars().take(60).collect()
    } else {
        title
    }
}

/// A message that opens with a question line (`Q: …` or `? …`): the yes/no
/// question it asks, and the text after that line, which is the state.
pub fn parse_question_line(text: &str) -> Option<(Question, String)> {
    let trimmed = text.trim_start();
    let (first, rest) = match trimmed.split_once('\n') {
        Some((first, rest)) => (first, rest),
        None => (trimmed, ""),
    };
    let first = first.trim();
    let asked = first
        .strip_prefix("Q:")
        .or_else(|| first.strip_prefix("q:"))
        .or_else(|| first.strip_prefix('?'))?
        .trim();
    if asked.is_empty() {
        return None;
    }
    Some((
        Question {
            instructions: asked.to_string(),
            kind: DecisionKind::Binary {},
            threshold: None,
        },
        rest.trim().to_string(),
    ))
}

/// What a decision chat asks of one message: the question, and the state.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatAsk {
    Ask {
        question: Question,
        state: String,
    },
    /// No question is set and the message names none, or there is no text to
    /// judge: the reply says how to ask.
    Explain(String),
}

/// The chat's question (from `decision_chat.v1`, else the message's own
/// question line) and the state it is asked about.
pub fn chat_ask(settings: Option<&DecisionChatSettings>, text: &str) -> ChatAsk {
    let (question, state) = match settings {
        Some(settings) => (settings.question.clone(), text.trim().to_string()),
        None => match parse_question_line(text) {
            Some(found) => found,
            None => return ChatAsk::Explain(how_to_ask()),
        },
    };
    if state.is_empty() {
        return ChatAsk::Explain(
            "Send the text to judge below the question line. The decision model answers the \
             question about that text."
                .to_string(),
        );
    }
    ChatAsk::Ask { question, state }
}

fn how_to_ask() -> String {
    "This chat's model is a decision model: it answers a question about the text you send, \
     with probabilities. Set the question in the composer, or start your message with a \
     question line, for example:\n\nQ: Does this note state the patient has multiple \
     sclerosis?\n<the note>"
        .to_string()
}

/// A model's short name for a reply line: the Baseten alias, else the model id.
fn model_label(model: &DecisionModelRef) -> String {
    decision_model_short_name(model)
}

/// The reply's text, readable on every surface: the answer, its probability,
/// whether it meets the question's threshold when there is one, and the model.
pub fn answer_text(decision: &Decision) -> String {
    let (answer, probability) = match &decision.answer {
        Answer::Binary { value } => (
            if *value { "Yes" } else { "No" }.to_string(),
            decision
                .probabilities
                .get(YES)
                .map(|p| format!("P(yes) {p:.2}")),
        ),
        Answer::Choice { option } => (
            option.clone(),
            decision
                .probabilities
                .get(option)
                .map(|p| format!("P({option}) {p:.2}")),
        ),
        Answer::Score { level, .. } => {
            let label = readable_answer(decision);
            (
                if label.starts_with("level ") {
                    format!("Level {level}")
                } else {
                    format!("Level {level}: {label}")
                },
                decision
                    .probabilities
                    .get(&level.to_string())
                    .map(|p| format!("P {p:.2}")),
            )
        }
    };
    let mut parts = vec![answer];
    parts.extend(probability);
    match (decision.passed, decision_pass_probability(decision)) {
        (Some(true), Some(p)) => parts.push(format!("meets the threshold (P {p:.2})")),
        (Some(false), Some(p)) => parts.push(format!("below the threshold (P {p:.2})")),
        _ => {}
    }
    if !decision.calibrated {
        parts.push("uncalibrated".to_string());
    }
    parts.push(model_label(&decision.model));
    parts.join(" · ")
}

/// What a decision chat replies with: its text and, when the model answered,
/// the decision and the id of the log row that records it.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionChatReply {
    pub text: String,
    /// The request's id, which is also its decision-log row's id
    /// ([`crate::decision::log::row_id`]), so a reload, a card or a link names
    /// the row without asking the log. A failed or refused request names its
    /// row too, which the cards draw as failed or refused. `None` when nothing
    /// was asked (no question, or a request that failed validation, which is
    /// not logged). A row whose write failed is a dangling id, and the
    /// surfaces then show this reply's text.
    pub decision_id: Option<String>,
    pub decision: Option<Decision>,
}

/// Answer one decision-chat message: ask `model` the chat's question about
/// `text` through `service` (Gate H, Crew, the size check and the log all
/// apply there).
///
/// `session` is the chat as sampled for this turn; it is logged with origin
/// `chat` and `turn_id`.
pub async fn run_decision_chat(
    service: &DecisionService,
    session: DecisionSession,
    model: &DecisionModelRef,
    settings: Option<&DecisionChatSettings>,
    text: &str,
) -> DecisionChatReply {
    let (question, state) = match chat_ask(settings, text) {
        ChatAsk::Ask { question, state } => (question, state),
        ChatAsk::Explain(text) => {
            return DecisionChatReply {
                text,
                decision_id: None,
                decision: None,
            }
        }
    };
    let session = session
        .with_origin(DecisionOrigin::Chat, None)
        .with_origin_label(Some("Decision chat".to_string()));
    let request = DecisionRequest {
        questions: IndexMap::from([(CHAT_QUESTION_NAME.to_string(), question)]),
        context: DecisionContext::Text(state),
    };
    // A request that fails validation is never logged, so its reply names no
    // row. Every later failure is logged under the id chosen here, so the
    // reply names that row and draws as a failed or refused card.
    if let Err(why) = request.validate() {
        return DecisionChatReply {
            text: format!("The decision model could not answer: {why}"),
            decision_id: None,
            decision: None,
        };
    }
    let request_id = uuid::Uuid::now_v7().to_string();
    match service
        .decide_as(Some(&session), Some(model), &request, &request_id)
        .await
    {
        Ok(decisions) => match decisions.into_iter().next() {
            Some(decision) => DecisionChatReply {
                text: answer_text(&decision),
                decision_id: Some(decision.request_id.clone()),
                decision: Some(decision),
            },
            None => DecisionChatReply {
                text: "The decision model returned no answer.".to_string(),
                decision_id: None,
                decision: None,
            },
        },
        Err(error) => DecisionChatReply {
            text: format!("The decision model could not answer: {error}"),
            decision_id: Some(request_id),
            decision: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::types::{DecisionUsage, ProbabilitySource, NO};

    fn binary_decision(p_yes: f64, threshold: Option<f64>) -> Decision {
        let passed = threshold.map(|t| p_yes >= t);
        Decision {
            question: CHAT_QUESTION_NAME.into(),
            answer: Answer::Binary {
                value: p_yes >= 0.5,
            },
            probabilities: IndexMap::from([
                (YES.to_string(), p_yes),
                (NO.to_string(), 1.0 - p_yes),
            ]),
            confidence: (2.0 * p_yes - 1.0).abs(),
            passed,
            pass_probability: threshold.map(|_| p_yes),
            source: ProbabilitySource::Native,
            calibrated: true,
            model: DecisionModelRef::new(OLLAMA_DECISION_PROVIDER, "tev1:0.8b"),
            tier: ProviderTier::Private,
            request_id: "req-1".into(),
            latency_ms: 22,
            usage: DecisionUsage::default(),
            raw: serde_json::Value::Null,
        }
    }

    #[test]
    fn a_reply_line_reads_everywhere() {
        assert_eq!(
            answer_text(&binary_decision(0.93, None)),
            "Yes · P(yes) 0.93 · tev1:0.8b"
        );
        assert_eq!(
            answer_text(&binary_decision(0.12, Some(0.8))),
            "No · P(yes) 0.12 · below the threshold (P 0.12) · tev1:0.8b"
        );
        let mut choice = binary_decision(0.0, None);
        choice.answer = Answer::Choice {
            option: "rrms".into(),
        };
        choice.probabilities = IndexMap::from([("rrms".into(), 0.81), ("spms".into(), 0.19)]);
        choice.calibrated = false;
        choice.model = DecisionModelRef::new(
            BASETEN_DECISION_PROVIDER,
            "decision-models/baseten-joey-2b-latest",
        );
        assert_eq!(
            answer_text(&choice),
            "rrms · P(rrms) 0.81 · uncalibrated · joey-1"
        );
        let mut score = binary_decision(0.0, None);
        score.answer = Answer::Score {
            level: 2,
            label: "moderate".into(),
            expected: 1.7,
        };
        score.probabilities =
            IndexMap::from([("0".into(), 0.1), ("1".into(), 0.26), ("2".into(), 0.64)]);
        assert_eq!(
            answer_text(&score),
            "Level 2: moderate · P 0.64 · tev1:0.8b"
        );
    }

    #[test]
    fn a_question_line_names_the_question_and_the_rest_is_the_state() {
        let (question, state) =
            parse_question_line("Q: Is an EDSS score stated?\nNeuro f/u. EDSS 3.5.").unwrap();
        assert_eq!(question.instructions, "Is an EDSS score stated?");
        assert_eq!(question.kind, DecisionKind::Binary {});
        assert_eq!(state, "Neuro f/u. EDSS 3.5.");
        assert!(parse_question_line("? Is it signed?\nyes it is").is_some());
        assert!(parse_question_line("Neuro f/u.").is_none());
        assert!(parse_question_line("Q:\nnothing asked").is_none());
    }

    #[test]
    fn the_chats_question_wins_and_a_message_without_one_is_explained() {
        let set = DecisionChatSettings {
            question: Question {
                instructions: "Does this note state MS?".into(),
                kind: DecisionKind::Binary {},
                threshold: None,
            },
        };
        assert_eq!(
            chat_ask(Some(&set), "Q: something else\nNote text"),
            ChatAsk::Ask {
                question: set.question.clone(),
                state: "Q: something else\nNote text".into()
            }
        );
        assert!(matches!(
            chat_ask(None, "Just a note."),
            ChatAsk::Explain(_)
        ));
        assert!(matches!(
            chat_ask(None, "Q: Is it signed?"),
            ChatAsk::Explain(_)
        ));
        assert!(matches!(chat_ask(Some(&set), "   "), ChatAsk::Explain(_)));
    }

    #[test]
    fn a_title_is_the_question_or_the_first_line() {
        assert_eq!(
            title_from("Q: Does this note state MS?\nlong note"),
            "Does this note state MS?"
        );
        assert_eq!(title_from("\n  First line here\nsecond"), "First line here");
        assert!(title_from(&"word ".repeat(40)).chars().count() <= 60);
    }

    #[tokio::test]
    async fn the_two_providers_declare_their_tiers_and_take_no_tools() {
        let baseten = BasetenDecisionChatProvider::with_model(ModelConfig::new_or_fail("joey-1"));
        assert_eq!(baseten.get_name(), BASETEN_DECISION_PROVIDER);
        assert_eq!(baseten.tier(), ProviderTier::Public);
        assert!(baseten.affiliation().is_none());
        assert!(!baseten.supports_tool_calls());
        assert_eq!(
            baseten.get_model_config().context_limit(),
            BASETEN_DECISION_CONTEXT
        );
        assert!(baseten.complete("", &[], &[]).await.is_err());
        let cloud =
            OllamaDecisionChatProvider::with_model(ModelConfig::new_or_fail("tev1:0.8b-cloud"));
        assert_eq!(cloud.tier(), ProviderTier::Public);
        assert_eq!(
            cloud.model_ref(),
            DecisionModelRef::new(OLLAMA_DECISION_PROVIDER, "tev1:0.8b-cloud")
        );
        assert!(is_decision_chat_provider(OLLAMA_DECISION_PROVIDER));
        assert!(!is_decision_chat_provider("ollama"));
        let metadata = OllamaDecisionChatProvider::metadata();
        assert_eq!(metadata.tier, ProviderTier::Private);
        assert!(metadata.runs_locally);
        assert_eq!(
            BasetenDecisionChatProvider::metadata().tier,
            ProviderTier::Public
        );
    }

    /// The window a decision chat's config carries is the model's own, not
    /// one figure for every Ollama model: the registry row when the server is
    /// not asked, and a window the config states is kept.
    #[test]
    fn a_decision_chats_window_is_its_models_own() {
        let tev = OllamaDecisionChatProvider::with_model(ModelConfig::new_or_fail("tev1:0.8b"));
        assert_eq!(tev.get_model_config().context_limit(), 2_048);
        let clef =
            OllamaDecisionChatProvider::with_model(ModelConfig::new_or_fail("clef-flash:latest"));
        assert_eq!(clef.get_model_config().context_limit(), 16_384);
        let unknown = OllamaDecisionChatProvider::with_model(ModelConfig::new_or_fail("nimble:9b"));
        assert_eq!(
            unknown.get_model_config().context_limit(),
            OLLAMA_DEFAULT_DECISION_CONTEXT
        );
        let stated = OllamaDecisionChatProvider::with_model(
            ModelConfig::new_or_fail("nimble:9b").with_context_limit(Some(4_096)),
        );
        assert_eq!(stated.get_model_config().context_limit(), 4_096);
        let joey = BasetenDecisionChatProvider::with_model(ModelConfig::new_or_fail("joey-1"));
        assert_eq!(joey.get_model_config().context_limit(), 16_000);
    }

    /// The Ollama decision provider declares no model list, so every picker
    /// asks what this Ollama has pulled (B-1: clef-flash:latest was missing
    /// from the chat's picker while the Monitors dialog listed it).
    #[tokio::test]
    async fn the_ollama_decision_models_listed_are_the_ones_discovered() {
        use wiremock::matchers::{body_partial_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        assert!(OllamaDecisionChatProvider::metadata()
            .known_models
            .is_empty());
        assert!(OllamaDecisionChatProvider::metadata().allows_unlisted_models);

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "models": [
                    {"name": "clef-flash:latest", "digest": "b1-clef"},
                    {"name": "tev1:0.8b", "digest": "b1-tev"},
                    {"name": "qwen3.8:27b", "digest": "b1-qwen"}
                ]
            })))
            .mount(&server)
            .await;
        for (name, capabilities, num_ctx) in [
            (
                "clef-flash:latest",
                vec!["decision", "vision"],
                "num_ctx 16384",
            ),
            ("tev1:0.8b", vec!["decision"], "num_ctx 2048"),
            ("qwen3.8:27b", vec!["completion", "tools"], ""),
        ] {
            Mock::given(method("POST"))
                .and(path("/api/show"))
                .and(body_partial_json(serde_json::json!({"model": name})))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "capabilities": capabilities,
                    "parameters": num_ctx,
                })))
                .mount(&server)
                .await;
        }
        let base = url::Url::parse(&server.uri()).unwrap();
        assert_eq!(
            ollama_decision_chat_models(&base).await,
            ["tev1:0.8b", "clef-flash:latest"]
        );
    }
}
