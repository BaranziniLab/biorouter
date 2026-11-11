//! The System One client: `POST {base}/v1/systemone`, one request carrying
//! every question over one shared state. One adapter serves Baseten (bearer
//! `BASETEN_API_KEY`) and Ollama 0.35 and later (no auth); llama.cpp's
//! `llama-server` speaks the same protocol and can be added with a base URL.
//!
//! The question builders, the answer-type check and the rule never to echo an
//! error body come from Albert Lee's prototype in PR #469
//! (`agents/baseten_decisions.rs`). Changed from it: every question in one
//! request, option descriptions separate from names, 1-level scores, scalar
//! text wrapped as `{"text": ...}`, the whole distribution kept, the host taken
//! from config, and `error_type` kept from an error body that is otherwise
//! dropped.
//!
//! Contract: <https://docs.baseten.co/reference/inference-api/system-one>.

use std::time::Duration;

use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::{json, Map, Value};
use url::Url;

use crate::config::Config;
use crate::decision::error::{DecisionError, DecisionResult};
use crate::decision::model::{DecisionCaps, DecisionModel, DecisionModelEntry};
use crate::decision::types::{
    DecisionContext, DecisionKind, DecisionModelRef, DecisionRequest, DecisionUsage, ModelVerdicts,
    ProbabilitySource, Question, QuestionVerdict, NO, YES,
};
use crate::privacy::affiliation::ModelAffiliation;
use crate::privacy::ProviderTier;
use crate::providers::api_client::{ApiClient, AuthMethod, AuthProvider};

/// Relative, so a base URL with a path (a proxy prefix) keeps it.
pub const SYSTEM_ONE_PATH: &str = "v1/systemone";
pub const BASETEN_DECISION_HOST_KEY: &str = "BASETEN_DECISION_HOST";
pub const DEFAULT_BASETEN_DECISION_HOST: &str = "https://inference.baseten.co";
/// The same key the declarative `baseten` chat provider reads.
pub const BASETEN_API_KEY: &str = "BASETEN_API_KEY";
/// Ollama's System One body limit.
pub const OLLAMA_MAX_REQUEST_BYTES: usize = 64 * 1024;
/// How long Ollama keeps a decision model loaded after a request, matching the
/// chat provider's default so a monitor does not reload it at every boundary.
const OLLAMA_KEEP_ALIVE: &str = "30m";
/// A backstop under the service's own deadline, which is shorter.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(120);
/// Retries after a 429, each after the server's `Retry-After` (capped) or a
/// doubling delay.
const RATE_LIMIT_RETRIES: u32 = 2;
const MAX_RETRY_DELAY: Duration = Duration::from_secs(4);

/// Which server the adapter talks to. Only the wire extras differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemOneFlavor {
    Baseten,
    Ollama,
}

impl SystemOneFlavor {
    fn label(self) -> &'static str {
        match self {
            Self::Baseten => "Baseten",
            Self::Ollama => "Ollama",
        }
    }
}

struct NoAuth;

#[async_trait]
impl AuthProvider for NoAuth {
    async fn get_auth_header(&self) -> anyhow::Result<(String, String)> {
        Ok(("X-No-Auth".to_string(), "true".to_string()))
    }
}

/// A System One decision model.
pub struct SystemOneAdapter {
    client: ApiClient,
    flavor: SystemOneFlavor,
    entry: DecisionModelEntry,
}

impl SystemOneAdapter {
    /// An adapter over an existing client, for tests and for callers that
    /// built the client themselves.
    pub fn new(client: ApiClient, flavor: SystemOneFlavor, entry: DecisionModelEntry) -> Self {
        Self {
            client,
            flavor,
            entry,
        }
    }

    /// Baseten, with the key and host from config.
    pub fn baseten_from_config(entry: DecisionModelEntry) -> DecisionResult<Self> {
        let config = Config::global();
        let key: String = config.get_secret("BASETEN_API_KEY").map_err(|_| {
            DecisionError::NoModel(
                "Baseten decision models need a Baseten API key. Add BASETEN_API_KEY in \
                 Settings › Models."
                    .to_string(),
            )
        })?;
        let host = config
            .get_param::<String>("BASETEN_DECISION_HOST")
            .unwrap_or_else(|_| DEFAULT_BASETEN_DECISION_HOST.to_string());
        Self::baseten(&host, key, entry)
    }

    /// Baseten at `host` with `api_key`.
    pub fn baseten(host: &str, api_key: String, entry: DecisionModelEntry) -> DecisionResult<Self> {
        let host = parse_base(host)?;
        let client = ApiClient::with_timeout(
            host.to_string(),
            AuthMethod::BearerToken(api_key),
            CLIENT_TIMEOUT,
        )
        .map_err(|e| DecisionError::NoModel(format!("Baseten client could not start: {e}")))?;
        Ok(Self::new(client, SystemOneFlavor::Baseten, entry))
    }

    /// An Ollama server at `base_url`, which sends no credential.
    pub fn ollama(base_url: &Url, entry: DecisionModelEntry) -> DecisionResult<Self> {
        let client = ApiClient::with_timeout(
            base_url.to_string(),
            AuthMethod::Custom(Box::new(NoAuth)),
            CLIENT_TIMEOUT,
        )
        .map_err(|e| DecisionError::NoModel(format!("Ollama client could not start: {e}")))?;
        Ok(Self::new(client, SystemOneFlavor::Ollama, entry))
    }

    fn name(&self) -> String {
        format!(
            "{} decision model `{}`",
            self.flavor.label(),
            self.entry.display_name
        )
    }

    async fn post_with_retry(&self, body: &Value) -> DecisionResult<reqwest::Response> {
        let mut attempt = 0;
        loop {
            let response = self
                .client
                .response_post(SYSTEM_ONE_PATH, body)
                .await
                .map_err(|e| DecisionError::Backend {
                    model: self.entry.model.clone(),
                    status: None,
                    error_type: None,
                    message: super::unreachable_sentence(
                        &format!("{} on {}", self.entry.display_name, self.flavor.label()),
                        &e,
                    ),
                })?;
            if response.status().as_u16() != 429 || attempt >= RATE_LIMIT_RETRIES {
                return Ok(response);
            }
            let delay = retry_after(&response)
                .unwrap_or_else(|| Duration::from_millis(500 * 2u64.pow(attempt)))
                .min(MAX_RETRY_DELAY);
            attempt += 1;
            tokio::time::sleep(delay).await;
        }
    }

    /// The error for a non-2xx answer. Do not echo a remote error body:
    /// validation errors can contain the submitted state. Only a short
    /// `error_type` token is kept and, for Ollama, BioRouter's own reading of
    /// two refusals ([`ollama_refusal`]), whose advice names `entry.model` and
    /// never the body.
    fn error_for_status(&self, status: u16, body: &Value) -> DecisionError {
        if let Some(error) = self.limit_refusal(status, body) {
            return error;
        }
        let classified = match self.flavor {
            SystemOneFlavor::Ollama => ollama_refusal(status, body),
            SystemOneFlavor::Baseten => None,
        };
        let error_type =
            safe_error_type(body).or_else(|| classified.map(|kind| kind.error_type().to_string()));
        let message = match classified {
            Some(kind) => kind.sentence(&self.name(), &self.entry.model),
            None => status_sentence(&self.name(), status, error_type.as_deref()),
        };
        DecisionError::Backend {
            model: self.entry.model.clone(),
            status: Some(status),
            error_type,
            message,
        }
    }
}

impl SystemOneAdapter {
    /// The body this adapter sends for `request`.
    fn body_for(&self, request: &DecisionRequest) -> Value {
        let mut body = build_body(&self.entry.model, request);
        if self.flavor == SystemOneFlavor::Ollama {
            body["keep_alive"] = json!(OLLAMA_KEEP_ALIVE);
        }
        body
    }

    /// A refusal that says the request was over one of the model's limits
    /// ([`over_limit_refusal`]), as the error Biorouter's own check before
    /// sending gives for the same limit: the window as
    /// [`DecisionError::TooLarge`], the 64 KiB body as a sentence about bytes,
    /// a family's option limit as one about options.
    fn limit_refusal(&self, status: u16, body: &Value) -> Option<DecisionError> {
        if self.flavor == SystemOneFlavor::Ollama && status == 413 {
            return Some(DecisionError::Invalid(format!(
                "The request to {} was over its {OLLAMA_MAX_REQUEST_BYTES}-byte request limit. \
                 Split the text into smaller pieces and ask again.",
                self.name()
            )));
        }
        match over_limit_refusal(status, body)? {
            OverLimit::Window { tokens, limit } => Some(DecisionError::TooLarge {
                chunk: "The text".to_string(),
                tokens,
                usable: limit,
                limit,
                model: format!("`{}`", self.entry.display_name),
            }),
            OverLimit::ChoiceOptions { limit } => Some(DecisionError::Unsupported(format!(
                "{} takes at most {limit} options per choice question. Ask with fewer options.",
                self.name()
            ))),
        }
    }
}

/// A refusal of a request over one of the model's limits, read from the
/// numbers in the server's error text and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverLimit {
    /// The prompt was `tokens` tokens, over the `limit`-token window.
    Window { tokens: usize, limit: usize },
    /// A choice offered more than the `limit` options this model takes.
    ChoiceOptions { limit: usize },
}

/// Which [`OverLimit`] a 400, 413 or 422 is, from its `error` text. Measured
/// 2026-11-10, each server says the window its own way: Ollama's tev1
/// `prompt 0 has 2503 tokens; expected 1–2048 (input is never truncated)`,
/// Ollama's clef `Clef input has 18140 tokens; context limit is 16384`, and
/// Baseten (422) `OneJev prompt has 18102 tokens, exceeding max_len 16000;
/// prompts are never truncated`; tev1 refuses a 25-option choice with
/// `question "q": Tev1 supports at most 24 candidates`. Only the two numbers
/// are read: the text can quote a question name, and is never copied.
pub fn over_limit_refusal(status: u16, body: &Value) -> Option<OverLimit> {
    if !matches!(status, 400 | 413 | 422) {
        return None;
    }
    let text = ["error", "detail", "message"]
        .iter()
        .find_map(|key| body.get(*key).and_then(Value::as_str))?;
    if text.contains(" candidates") {
        if let Some(limit) = number_after(text, "supports at most ") {
            return Some(OverLimit::ChoiceOptions { limit });
        }
    }
    let tokens = number_after(text, " has ").filter(|_| text.contains(" tokens"))?;
    let limit = [
        "expected 1\u{2013}",
        "expected 1-",
        "context limit is ",
        "max_len ",
    ]
    .iter()
    .find_map(|marker| number_after(text, marker))?;
    Some(OverLimit::Window { tokens, limit })
}

/// The whole number that follows the first `marker` in `text`.
fn number_after(text: &str, marker: &str) -> Option<usize> {
    let (_, after) = text.split_once(marker)?;
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// An Ollama refusal BioRouter recognises from the error text, without copying
/// any of it. Carried in `DecisionError::Backend.error_type` under the
/// `biorouter.` prefix ([`OllamaRefusal::error_type`]), so a reader of the
/// error, or of a log line, never takes it for a token the server sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OllamaRefusal {
    /// The copy on disk is in a format this Ollama no longer loads (measured
    /// on Ollama 0.40.14: a decision model built from a GGUF before the format
    /// changed). Ollama checks this before it loads the model or reads the
    /// questions.
    FormatUnsupported,
    /// The model's capabilities lack `decision`.
    NotADecisionModel,
}

impl OllamaRefusal {
    pub fn error_type(self) -> &'static str {
        match self {
            Self::FormatUnsupported => "biorouter.model_format_unsupported",
            Self::NotADecisionModel => "biorouter.model_lacks_decision",
        }
    }

    /// What to tell the person: `name` is how the model is described
    /// ("Ollama decision model `x`"), `model` the name Ollama knows it by,
    /// which the commands use.
    pub fn sentence(self, name: &str, model: &str) -> String {
        match self {
            Self::FormatUnsupported => format!(
                "{name} cannot be loaded: Ollama says this copy is in a format it no longer \
                 supports. Re-download it with `ollama pull {model}`, or remove it with \
                 `ollama rm {model}`."
            ),
            Self::NotADecisionModel => format!(
                "{name} cannot answer through System One: Ollama says this model does not \
                 support decisions (its capabilities lack `decision`). Ask with the `logprob` \
                 adapter instead."
            ),
        }
    }
}

/// Which [`OllamaRefusal`] an Ollama error answer is, read from the `error`
/// text of a 400. Only the advice is chosen from it; nothing is copied, so a
/// body that happened to contain the phrase can give the wrong advice but can
/// never leak what it quoted.
pub fn ollama_refusal(status: u16, body: &Value) -> Option<OllamaRefusal> {
    if status != 400 {
        return None;
    }
    let text = body.get("error")?.as_str()?;
    if text.contains("no longer supported") {
        Some(OllamaRefusal::FormatUnsupported)
    } else if text.contains("does not support decision") {
        Some(OllamaRefusal::NotADecisionModel)
    } else {
        None
    }
}

/// The sentence a model Ollama will not load is listed with: the same one a
/// request to it fails with.
pub fn ollama_load_refusal_sentence(model: &str) -> String {
    OllamaRefusal::FormatUnsupported.sentence(
        &format!(
            "{} decision model `{model}`",
            SystemOneFlavor::Ollama.label()
        ),
        model,
    )
}

fn parse_base(host: &str) -> DecisionResult<Url> {
    let url = Url::parse(host.trim()).map_err(|_| {
        DecisionError::Invalid(format!(
            "BASETEN_DECISION_HOST must be an http(s) URL such as {DEFAULT_BASETEN_DECISION_HOST}"
        ))
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(DecisionError::Invalid(format!(
            "BASETEN_DECISION_HOST must be an http(s) URL such as {DEFAULT_BASETEN_DECISION_HOST}"
        )));
    }
    Ok(url)
}

fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    let value = response.headers().get("retry-after")?.to_str().ok()?;
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

#[async_trait]
impl DecisionModel for SystemOneAdapter {
    fn model_ref(&self) -> DecisionModelRef {
        self.entry.model_ref()
    }

    fn display_name(&self) -> String {
        self.entry.display_name.clone()
    }

    fn tier(&self) -> ProviderTier {
        self.entry.tier
    }

    fn affiliation(&self) -> Option<ModelAffiliation> {
        self.entry.affiliation
    }

    fn caps(&self) -> DecisionCaps {
        system_one_caps(&self.entry, self.flavor)
    }

    fn max_choice_options(&self) -> usize {
        super::catalog::choice_option_limit(&self.entry.model_ref())
    }

    /// The encoded body, as sent: Ollama's 64 KiB is a limit on it, and JSON
    /// escaping (a newline or a quote is two bytes), the model name and
    /// `keep_alive` make it larger than the state's own bytes. Measured
    /// 2026-11-10, a state that passed a count of the raw bytes was refused
    /// with HTTP 413.
    fn request_bytes(&self, request: &DecisionRequest) -> usize {
        serde_json::to_vec(&self.body_for(request)).map_or(usize::MAX, |body| body.len())
    }

    async fn decide(&self, request: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
        let body = self.body_for(request);
        let response = self.post_with_retry(&body).await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.json::<Value>().await.unwrap_or(Value::Null);
            return Err(self.error_for_status(status.as_u16(), &body));
        }
        // A body that is not JSON was billed too, but says for how much in no
        // form that can be read, so nothing is counted and nothing estimated.
        let payload: Value = response
            .json()
            .await
            .map_err(|_| DecisionError::malformed(&self.entry.model, "the body was not JSON"))?;
        // Read before the answers are judged: the model answered, so the
        // request was billed whether or not its answers can be used, and an
        // answer that cannot be read still carries what it cost.
        let usage = reported_usage(&payload);
        parse_response(request, &payload).map_err(|detail| {
            DecisionError::malformed(&self.entry.model, detail).with_billed_usage(usage)
        })
    }
}

/// The capabilities a System One entry has on this server.
pub fn system_one_caps(entry: &DecisionModelEntry, flavor: SystemOneFlavor) -> DecisionCaps {
    DecisionCaps {
        kinds: entry.kinds.clone(),
        max_questions: crate::decision::types::MAX_QUESTIONS,
        context_tokens: entry.context_tokens,
        max_concurrency: if entry.local {
            crate::decision::model::LOCAL_CONCURRENCY
        } else {
            crate::decision::model::REMOTE_CONCURRENCY
        },
        max_request_bytes: (flavor == SystemOneFlavor::Ollama).then_some(OLLAMA_MAX_REQUEST_BYTES),
        local: entry.local,
    }
}

/// `error_type` from an error body, only when it is a short identifier, so a
/// server that put the state in it cannot get it echoed.
fn safe_error_type(body: &Value) -> Option<String> {
    let value = body.get("error_type")?.as_str()?;
    (!value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'))
    .then(|| value.to_string())
}

fn status_sentence(name: &str, status: u16, error_type: Option<&str>) -> String {
    let kind = error_type.map(|t| format!(" ({t})")).unwrap_or_default();
    match status {
        401 | 403 => format!(
            "{name} refused the credential (HTTP {status}{kind}). Check the API key in Settings › Models."
        ),
        404 => format!("{name} was not found on the server (HTTP 404{kind})."),
        400 | 413 | 422 => format!(
            "{name} refused the request (HTTP {status}{kind}). System One never truncates, so a \
             state over the model's window fails this way; a malformed question does too."
        ),
        429 => format!("{name} is rate limited (HTTP 429{kind}). Try again shortly."),
        _ => format!("{name} failed (HTTP {status}{kind})."),
    }
}

/// System One's type name for a question kind.
pub fn wire_type(kind: &DecisionKind) -> &'static str {
    match kind {
        DecisionKind::Binary {} => "noul",
        DecisionKind::Choice { .. } => "choice",
        DecisionKind::Score { .. } => "score",
    }
}

/// One question on the wire: `type`, `instructions` and, where the type takes
/// them, `criteria`. A choice's criteria map each option name to its
/// description (the name itself when there is none); a score's are its levels,
/// lowest first; a noul's are optional and omitted.
pub fn build_question(question: &Question) -> Value {
    let mut wire = json!({
        "type": wire_type(&question.kind),
        "instructions": question.instructions,
    });
    match &question.kind {
        DecisionKind::Binary {} => {}
        DecisionKind::Choice { options } => {
            let criteria: Map<String, Value> = options
                .iter()
                .map(|(name, description)| {
                    let text = description
                        .as_deref()
                        .map(str::trim)
                        .filter(|d| !d.is_empty())
                        .unwrap_or(name);
                    (name.clone(), json!(text))
                })
                .collect();
            wire["criteria"] = Value::Object(criteria);
        }
        DecisionKind::Score { levels } => wire["criteria"] = json!(levels),
    }
    wire
}

/// The state on the wire. System One takes a string, an object, an array or
/// `{"messages":[...]}`, and rejects a top-level number, boolean or null. Text
/// that reads as such a scalar (`42`, `true`), and a JSON scalar, are wrapped
/// as `{"text": ...}`, so whatever a server does with a string state, it never
/// sees a bare scalar.
pub fn build_state(context: &DecisionContext) -> Value {
    match context {
        DecisionContext::Text(text) => {
            let scalar = serde_json::from_str::<Value>(text.trim())
                .map(|v| matches!(v, Value::Number(_) | Value::Bool(_) | Value::Null))
                .unwrap_or(false);
            if scalar {
                json!({ "text": text })
            } else {
                Value::String(text.clone())
            }
        }
        DecisionContext::Json(value) => match value {
            Value::Number(_) | Value::Bool(_) | Value::Null => json!({ "text": value }),
            other => other.clone(),
        },
        DecisionContext::Messages(messages) => json!({
            "messages": messages
                .iter()
                .map(|m| json!({"role": m.role, "content": m.content}))
                .collect::<Vec<_>>()
        }),
    }
}

/// The whole request body.
pub fn build_body(model: &str, request: &DecisionRequest) -> Value {
    let questions: Map<String, Value> = request
        .questions
        .iter()
        .map(|(name, question)| (name.clone(), build_question(question)))
        .collect();
    json!({
        "model": model,
        "state": build_state(&request.context),
        "questions": Value::Object(questions),
    })
}

fn probability(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .filter(|p| p.is_finite() && (0.0..=1.0 + 1e-9).contains(p))
}

/// Read a System One response into verdicts, in question order. A missing
/// answer, an answer of the wrong type, or a choice or score without its
/// probabilities is an error, never a guessed 1.0.
pub fn parse_response(request: &DecisionRequest, payload: &Value) -> Result<ModelVerdicts, String> {
    let answers = payload
        .get("answers")
        .and_then(Value::as_object)
        .ok_or("the response has no `answers` object")?;
    let mut verdicts = Vec::with_capacity(request.questions.len());
    for (name, question) in &request.questions {
        let answer = answers
            .get(name)
            .ok_or_else(|| format!("no answer for question `{name}`"))?;
        verdicts.push(parse_answer(name, question, answer)?);
    }
    Ok(ModelVerdicts {
        verdicts,
        usage: reported_usage(payload),
        served_model: served_model(payload),
    })
}

/// The tokens a 2xx System One body says the request was billed for: its
/// `usage.input_tokens` and `usage.output_tokens`, read whether or not the
/// answers can be. A count that is missing, not a whole number or below zero
/// is `None`, never a guess, and a body with no `usage` gives the default.
pub fn reported_usage(payload: &Value) -> DecisionUsage {
    let usage = payload.get("usage");
    let count = |key: &str| {
        usage
            .and_then(|u| u.get(key))
            .and_then(Value::as_i64)
            .filter(|n| *n >= 0)
    };
    DecisionUsage {
        input_tokens: count("input_tokens"),
        output_tokens: count("output_tokens"),
    }
}

/// The model the server says answered, which can differ from the one asked.
fn served_model(payload: &Value) -> Option<String> {
    payload
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn parse_answer(
    name: &str,
    question: &Question,
    answer: &Value,
) -> Result<QuestionVerdict, String> {
    let expected = wire_type(&question.kind);
    let got = answer.get("type").and_then(Value::as_str).unwrap_or("none");
    if got != expected {
        return Err(format!(
            "question `{name}` is a `{expected}` question, but the answer is `{got}`"
        ));
    }
    let mut reported_choice = None;
    let probabilities: IndexMap<String, f64> = match &question.kind {
        DecisionKind::Binary {} => {
            let p = answer
                .get("noul")
                .and_then(probability)
                .ok_or_else(|| format!("question `{name}`: `noul` is not a probability"))?;
            let p = p.min(1.0);
            IndexMap::from([(YES.to_string(), p), (NO.to_string(), 1.0 - p)])
        }
        DecisionKind::Choice { .. } => {
            reported_choice = answer
                .get("choice")
                .and_then(Value::as_str)
                .map(str::to_string);
            read_distribution(name, answer, |key| Some(key.to_string()))?
        }
        DecisionKind::Score { levels } => {
            // Keys are level numbers ("0", "1", ...). A server that keys them by
            // label is read through the levels, or through its own legend.
            let legend = answer.get("legend").and_then(Value::as_object);
            read_distribution(name, answer, |key| {
                if key.parse::<usize>().is_ok() {
                    return Some(key.to_string());
                }
                levels
                    .iter()
                    .position(|level| level == key)
                    .or_else(|| {
                        legend.and_then(|legend| {
                            legend
                                .iter()
                                .find(|(_, label)| label.as_str() == Some(key))
                                .and_then(|(index, _)| index.parse::<usize>().ok())
                        })
                    })
                    .map(|i| i.to_string())
            })?
        }
    };
    Ok(QuestionVerdict {
        question: name.to_string(),
        probabilities,
        source: ProbabilitySource::Native,
        calibrated: true,
        reported_choice,
        raw: answer.clone(),
    })
}

fn read_distribution(
    name: &str,
    answer: &Value,
    key_of: impl Fn(&str) -> Option<String>,
) -> Result<IndexMap<String, f64>, String> {
    let reported = answer
        .get("probabilities")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("question `{name}`: the answer has no `probabilities`"))?;
    let mut out = IndexMap::new();
    for (key, value) in reported {
        let key =
            key_of(key).ok_or_else(|| format!("question `{name}`: unknown outcome `{key}`"))?;
        let p = probability(value)
            .ok_or_else(|| format!("question `{name}`: a probability is outside 0 to 1"))?;
        *out.entry(key).or_insert(0.0) += p;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::types::{ContextMessage, Threshold};

    fn questions() -> IndexMap<String, Question> {
        IndexMap::from([
            (
                "has_ms".to_string(),
                Question {
                    instructions: "Does this note state the patient has multiple sclerosis?".into(),
                    kind: DecisionKind::Binary {},
                    threshold: Some(Threshold::Yes(0.9)),
                },
            ),
            (
                "course".to_string(),
                Question {
                    instructions: "Which MS course is described?".into(),
                    kind: DecisionKind::Choice {
                        options: IndexMap::from([
                            ("rrms".to_string(), Some("Relapsing-remitting".to_string())),
                            (
                                "spms".to_string(),
                                Some("Secondary progressive".to_string()),
                            ),
                            ("none".to_string(), None),
                        ]),
                    },
                    threshold: None,
                },
            ),
            (
                "edss".to_string(),
                Question {
                    instructions: "How much disability is described?".into(),
                    kind: DecisionKind::Score {
                        levels: vec!["None".into()],
                    },
                    threshold: None,
                },
            ),
        ])
    }

    fn request(context: DecisionContext) -> DecisionRequest {
        DecisionRequest {
            questions: questions(),
            context,
        }
    }

    #[test]
    fn every_question_goes_in_one_body_with_its_criteria() {
        let body = build_body(
            "tev1:0.8b",
            &request(DecisionContext::Text("Neuro f/u.".into())),
        );
        assert_eq!(body["model"], "tev1:0.8b");
        assert_eq!(body["state"], "Neuro f/u.");
        assert_eq!(
            body["questions"]["has_ms"],
            json!({"type": "noul", "instructions": "Does this note state the patient has multiple sclerosis?"})
        );
        // Descriptions are sent as descriptions, and a missing one sends the name.
        assert_eq!(
            body["questions"]["course"]["criteria"],
            json!({"rrms": "Relapsing-remitting", "spms": "Secondary progressive", "none": "none"})
        );
        // A one-level score is sent as the API allows.
        assert_eq!(
            body["questions"]["edss"],
            json!({"type": "score", "instructions": "How much disability is described?", "criteria": ["None"]})
        );
        // The threshold is BioRouter's business, not the wire's.
        assert!(body["questions"]["has_ms"].get("threshold").is_none());
        let names: Vec<_> = body["questions"].as_object().unwrap().keys().collect();
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn scalar_text_is_wrapped_and_other_state_is_sent_as_is() {
        assert_eq!(
            build_state(&DecisionContext::Text("42".into())),
            json!({"text": "42"})
        );
        assert_eq!(
            build_state(&DecisionContext::Text(" true ".into())),
            json!({"text": " true "})
        );
        assert_eq!(
            build_state(&DecisionContext::Text("null".into())),
            json!({"text": "null"})
        );
        // Text that merely contains JSON stays text.
        assert_eq!(
            build_state(&DecisionContext::Text("{\"a\":1}".into())),
            json!("{\"a\":1}")
        );
        assert_eq!(
            build_state(&DecisionContext::Json(json!(3.5))),
            json!({"text": 3.5})
        );
        assert_eq!(
            build_state(&DecisionContext::Json(json!(false))),
            json!({"text": false})
        );
        assert_eq!(
            build_state(&DecisionContext::Json(json!({"a": 1}))),
            json!({"a": 1})
        );
        assert_eq!(
            build_state(&DecisionContext::Json(json!([1, 2]))),
            json!([1, 2])
        );
        assert_eq!(
            build_state(&DecisionContext::Messages(vec![ContextMessage {
                role: "user".into(),
                content: "hi".into()
            }])),
            json!({"messages": [{"role": "user", "content": "hi"}]})
        );
    }

    #[test]
    fn answers_keep_the_whole_distribution_and_usage() {
        let payload = json!({
            "model": "backend-id-may-differ",
            "answers": {
                "has_ms": {"type": "noul", "noul": 0.97},
                "course": {"type": "choice", "choice": "rrms",
                           "probabilities": {"rrms": 0.93, "spms": 0.05, "none": 0.02},
                           "confidence": 0.9},
                "edss": {"type": "score", "score": 0.0, "probabilities": {"0": 1.0},
                         "legend": {"0": "None"}, "confidence": 0.0}
            },
            "usage": {"input_tokens": 180, "output_tokens": 3}
        });
        let parsed = parse_response(&request(DecisionContext::Text("s".into())), &payload).unwrap();
        assert_eq!(
            parsed.served_model.as_deref(),
            Some("backend-id-may-differ")
        );
        assert_eq!(parsed.usage.input_tokens, Some(180));
        assert_eq!(parsed.usage.output_tokens, Some(3));
        let has_ms = &parsed.verdicts[0];
        assert!((has_ms.probabilities[YES] - 0.97).abs() < 1e-12);
        assert!((has_ms.probabilities[NO] - 0.03).abs() < 1e-12);
        assert_eq!(has_ms.source, ProbabilitySource::Native);
        assert!(has_ms.calibrated);
        let course = &parsed.verdicts[1];
        assert_eq!(course.reported_choice.as_deref(), Some("rrms"));
        assert_eq!(course.probabilities["spms"], 0.05);
        // The native confidence is kept in raw, not used.
        assert_eq!(course.raw["confidence"], 0.9);
        assert_eq!(parsed.verdicts[2].probabilities["0"], 1.0);
    }

    #[test]
    fn a_wrong_or_incomplete_answer_is_refused_not_guessed() {
        let req = request(DecisionContext::Text("s".into()));
        let mismatch = json!({"answers": {
            "has_ms": {"type": "choice", "choice": "continue"},
            "course": {"type": "choice", "choice": "rrms", "probabilities": {"rrms": 1.0}},
            "edss": {"type": "score", "probabilities": {"0": 1.0}}
        }});
        let err = parse_response(&req, &mismatch).unwrap_err();
        assert!(err.contains("`noul`") && err.contains("`choice`"), "{err}");

        let missing = json!({"answers": {"has_ms": {"type": "noul", "noul": 0.5}}});
        assert!(parse_response(&req, &missing)
            .unwrap_err()
            .contains("course"));

        let no_probabilities = json!({"answers": {
            "has_ms": {"type": "noul", "noul": 0.5},
            "course": {"type": "choice", "choice": "rrms"},
            "edss": {"type": "score", "probabilities": {"0": 1.0}}
        }});
        assert!(parse_response(&req, &no_probabilities)
            .unwrap_err()
            .contains("probabilities"));

        let out_of_range = json!({"answers": {
            "has_ms": {"type": "noul", "noul": 1.7},
            "course": {"type": "choice", "probabilities": {"rrms": 1.0}},
            "edss": {"type": "score", "probabilities": {"0": 1.0}}
        }});
        assert!(parse_response(&req, &out_of_range).is_err());
        assert!(parse_response(&req, &json!({"error": "x"})).is_err());
    }

    /// The answer is judged unreadable, but the request was billed: its usage
    /// is read all the same, and only a count that is really there is kept.
    #[test]
    fn usage_is_read_even_when_the_answers_cannot_be() {
        let req = request(DecisionContext::Text("s".into()));
        let wrong_type = json!({
            "answers": {"has_ms": {"type": "choice", "choice": "continue"}},
            "usage": {"input_tokens": 42, "output_tokens": 1}
        });
        assert!(parse_response(&req, &wrong_type).is_err());
        assert_eq!(
            reported_usage(&wrong_type),
            DecisionUsage {
                input_tokens: Some(42),
                output_tokens: Some(1)
            }
        );
        // No answers at all, and still a usage.
        let no_answers = json!({"usage": {"input_tokens": 7, "output_tokens": 0}});
        assert_eq!(
            reported_usage(&no_answers),
            DecisionUsage {
                input_tokens: Some(7),
                output_tokens: Some(0)
            }
        );
        // No usage: nothing is invented.
        assert_eq!(
            reported_usage(&json!({"answers": {}})),
            DecisionUsage::default()
        );
        // A count that is not a whole number of tokens is not a count.
        assert_eq!(
            reported_usage(&json!({"usage": {"input_tokens": "42", "output_tokens": 1.5}})),
            DecisionUsage::default()
        );
        assert_eq!(
            reported_usage(&json!({"usage": {"input_tokens": -3, "output_tokens": 2}})),
            DecisionUsage {
                input_tokens: None,
                output_tokens: Some(2)
            }
        );
    }

    #[test]
    fn score_probabilities_keyed_by_label_are_read_through_the_levels() {
        let req = DecisionRequest {
            questions: IndexMap::from([(
                "q".to_string(),
                Question {
                    instructions: "Rate".into(),
                    kind: DecisionKind::Score {
                        levels: vec!["Poor".into(), "Good".into()],
                    },
                    threshold: None,
                },
            )]),
            context: DecisionContext::Text("s".into()),
        };
        let payload = json!({"answers": {"q": {"type": "score", "score": 0.8,
            "probabilities": {"Poor": 0.2, "Good": 0.8}}}});
        let parsed = parse_response(&req, &payload).unwrap();
        assert_eq!(parsed.verdicts[0].probabilities["1"], 0.8);
        let unknown = json!({"answers": {"q": {"type": "score", "probabilities": {"Great": 1.0}}}});
        assert!(parse_response(&req, &unknown).is_err());
    }

    #[test]
    fn only_a_short_error_type_survives_an_error_body() {
        assert_eq!(
            safe_error_type(&json!({"error": "SENSITIVE STATE", "error_type": "invalid_request"})),
            Some("invalid_request".to_string())
        );
        assert_eq!(
            safe_error_type(&json!({"error_type": "the patient said something"})),
            None
        );
        assert_eq!(safe_error_type(&json!({"error": "x"})), None);
        let sentence = status_sentence("Baseten decision model `joey-1`", 413, Some("too_large"));
        assert!(sentence.contains("413") && sentence.contains("too_large"));
    }

    /// Ollama 0.40.14's answer for a decision model built from a GGUF before
    /// the format change (measured 2026-11-05, `clef-flash:latest`).
    fn format_refusal_body() -> Value {
        json!({"error": "'clef-flash:latest' was downloaded in a format that is no longer \
                         supported. To re-download, run 'ollama pull clef-flash:latest'"})
    }

    #[test]
    fn an_ollama_format_refusal_names_the_fix_and_never_echoes_the_body() {
        assert_eq!(
            ollama_refusal(400, &format_refusal_body()),
            Some(OllamaRefusal::FormatUnsupported)
        );
        // Only a 400 carries it, and the other 400s keep the generic sentence.
        assert_eq!(ollama_refusal(422, &format_refusal_body()), None);
        assert_eq!(
            ollama_refusal(400, &json!({"error": "prompt 0 has 3309 tokens"})),
            None
        );
        assert_eq!(
            ollama_refusal(
                400,
                &json!({"error": "questions must contain 1\u{2013}64 fields"})
            ),
            None
        );
        assert_eq!(ollama_refusal(400, &Value::Null), None);
        assert_eq!(
            ollama_refusal(
                400,
                &json!({"error": "registry.ollama.ai/library/qwen3:8b does not support decision"})
            ),
            Some(OllamaRefusal::NotADecisionModel)
        );

        let name = "Ollama decision model `clef-flash:latest`";
        let sentence = OllamaRefusal::FormatUnsupported.sentence(name, "clef-flash:latest");
        assert!(
            sentence.contains("ollama pull clef-flash:latest")
                && sentence.contains("ollama rm clef-flash:latest")
                && sentence.starts_with(name),
            "{sentence}"
        );
        assert!(!sentence.contains("never truncates"), "{sentence}");
        assert!(!sentence.contains("was downloaded"), "{sentence}");
        assert_eq!(
            ollama_load_refusal_sentence("clef-flash:latest"),
            sentence,
            "a listing and a failed request say the same thing"
        );
        // BioRouter's own classification is never spelled like a server token.
        assert!(OllamaRefusal::FormatUnsupported
            .error_type()
            .starts_with("biorouter."));
        assert!(OllamaRefusal::NotADecisionModel
            .error_type()
            .starts_with("biorouter."));
    }

    /// The three servers' over-window refusals, as measured 2026-11-10, all
    /// read as one: the two numbers, nothing else of the text.
    #[test]
    fn every_servers_over_window_refusal_reads_as_the_same_two_numbers() {
        let tev = json!({"error": "prompt 0 has 2503 tokens; expected 1\u{2013}2048 (input is \
                                    never truncated)"});
        let clef = json!({"error": "Clef input has 18140 tokens; context limit is 16384"});
        let baseten = json!({"error": "OneJev prompt has 18102 tokens, exceeding max_len 16000; \
                                        prompts are never truncated", "error_type": "Validation"});
        assert_eq!(
            over_limit_refusal(400, &tev),
            Some(OverLimit::Window {
                tokens: 2503,
                limit: 2048
            })
        );
        assert_eq!(
            over_limit_refusal(400, &clef),
            Some(OverLimit::Window {
                tokens: 18140,
                limit: 16384
            })
        );
        assert_eq!(
            over_limit_refusal(422, &baseten),
            Some(OverLimit::Window {
                tokens: 18102,
                limit: 16000
            })
        );
        assert_eq!(
            over_limit_refusal(
                400,
                &json!({"error": "question \"q\": Tev1 supports at most 24 candidates"})
            ),
            Some(OverLimit::ChoiceOptions { limit: 24 })
        );
        // Anything else keeps the generic sentence.
        assert_eq!(over_limit_refusal(500, &tev), None);
        assert_eq!(over_limit_refusal(400, &format_refusal_body()), None);
        assert_eq!(
            over_limit_refusal(400, &json!({"error": "prompt 0 has 3309 tokens"})),
            None
        );
        assert_eq!(over_limit_refusal(400, &Value::Null), None);
    }

    fn adapter(flavor: SystemOneFlavor) -> SystemOneAdapter {
        let entry = DecisionModelEntry {
            provider: "ollama_decision".to_string(),
            model: "tev1:0.8b".to_string(),
            display_name: "tev1:0.8b".to_string(),
            context_tokens: 2048,
            kinds: vec![crate::decision::types::DecisionKindTag::Binary],
            tier: ProviderTier::Private,
            affiliation: None,
            adapter: crate::decision::types::DecisionAdapter::SystemOne,
            local: true,
            load_refusal: None,
        };
        let client = ApiClient::new(
            "http://127.0.0.1:9/".to_string(),
            AuthMethod::Custom(Box::new(NoAuth)),
        )
        .unwrap();
        SystemOneAdapter::new(client, flavor, entry)
    }

    #[test]
    fn a_limit_refusal_says_which_limit_and_never_echoes_the_body() {
        let model = adapter(SystemOneFlavor::Ollama);
        let window = model.error_for_status(
            400,
            &json!({"error": "prompt 0 has 2503 tokens; expected 1\u{2013}2048 SECRET"}),
        );
        assert!(
            matches!(
                &window,
                DecisionError::TooLarge { tokens: 2503, limit: 2048, model, .. }
                    if model == "`tev1:0.8b`"
            ),
            "{window:?}"
        );
        assert!(!window.to_string().contains("SECRET"));
        let bytes = model.error_for_status(
            413,
            &json!({"error": "text and schema must not exceed 64 KiB"}),
        );
        assert!(
            matches!(&bytes, DecisionError::Invalid(text)
                if text.contains("65536-byte request limit") && !text.contains("window")),
            "{bytes:?}"
        );
        let options = model.error_for_status(
            400,
            &json!({"error": "question \"SECRET\": Tev1 supports at most 24 candidates"}),
        );
        assert!(
            matches!(&options, DecisionError::Unsupported(text)
                if text.contains("at most 24 options") && !text.contains("SECRET")),
            "{options:?}"
        );
        // A 413 from Baseten is not Ollama's body limit.
        let baseten = adapter(SystemOneFlavor::Baseten).error_for_status(413, &json!({}));
        assert!(
            matches!(baseten, DecisionError::Backend { .. }),
            "{baseten:?}"
        );
    }

    #[test]
    fn the_bytes_counted_are_the_encoded_body() {
        let model = adapter(SystemOneFlavor::Ollama);
        let mut questions = IndexMap::new();
        questions.insert(
            "q".to_string(),
            Question {
                instructions: "Is it there?".to_string(),
                kind: DecisionKind::Binary {},
                threshold: None,
            },
        );
        // Every newline and quote is two bytes once escaped.
        let text = format!("A note.{}", "\n\"".repeat(1_000));
        let request = DecisionRequest {
            questions,
            context: DecisionContext::Text(text.clone()),
        };
        let sent = serde_json::to_vec(&model.body_for(&request)).unwrap().len();
        assert_eq!(model.request_bytes(&request), sent);
        assert!(model.request_bytes(&request) > text.len() + 2_000);
        assert_eq!(model.max_choice_options(), 24);
    }

    #[test]
    fn a_refused_credential_and_a_server_fault_have_their_own_sentences() {
        let name = "Baseten decision model `joey-1`";
        let refused = status_sentence(name, 401, None);
        assert!(
            refused.contains("refused the credential (HTTP 401)"),
            "{refused}"
        );
        assert!(status_sentence(name, 403, None).contains("refused the credential (HTTP 403)"));
        let fault = status_sentence(name, 503, None);
        assert_eq!(fault, format!("{name} failed (HTTP 503)."));
        assert!(status_sentence(name, 429, None).contains("rate limited"));
    }

    #[test]
    fn the_baseten_host_must_be_a_url() {
        assert!(parse_base("https://inference.baseten.co").is_ok());
        assert!(parse_base("inference.baseten.co").is_err());
        assert!(parse_base("file:///etc/passwd").is_err());
    }
}
