use super::{anthropic, google};
use crate::conversation::message::Message;
use crate::model::ModelConfig;
use crate::providers::base::Usage;
use anyhow::{Context, Result};
use rmcp::model::Tool;
use serde_json::Value;

use std::fmt;

pub type StreamingMessageStream = std::pin::Pin<
    Box<
        dyn futures::Stream<Item = anyhow::Result<crate::providers::base::ProviderStreamItem>>
            + Send
            + 'static,
    >,
>;

/// Sensible default values of Google Cloud Platform (GCP) locations for model deployment.
///
/// `Iowa` and `Ohio` are single regions; `Global` is Vertex's global endpoint
/// (host `aiplatform.googleapis.com`, path `locations/global`), the only place
/// some models are served at all — see [`ModelAvailability`].
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum GcpLocation {
    /// Represents the us-central1 region in Iowa
    Iowa,
    /// Represents the us-east5 region in Ohio
    Ohio,
    /// Vertex's global endpoint (`locations/global`)
    Global,
}

impl fmt::Display for GcpLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Iowa => write!(f, "us-central1"),
            Self::Ohio => write!(f, "us-east5"),
            Self::Global => write!(f, "{GLOBAL_LOCATION}"),
        }
    }
}

impl TryFrom<&str> for GcpLocation {
    type Error = ModelError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "us-central1" => Ok(Self::Iowa),
            "us-east5" => Ok(Self::Ohio),
            GLOBAL_LOCATION => Ok(Self::Global),
            _ => Err(ModelError::UnsupportedLocation(s.to_string())),
        }
    }
}

/// Vertex's name for its global endpoint location.
pub const GLOBAL_LOCATION: &str = "global";
/// Vertex's multi-region locations. Each serves the models that are not in any
/// single region while keeping requests inside that geography; they are used
/// only when the user configures one as `GCP_LOCATION`.
pub const MULTI_REGION_LOCATIONS: &[&str] = &["us", "eu"];

/// Where Vertex serves a model — which decides where a request must go.
///
/// Checked against the "Supported regions" tables of the Vertex model pages
/// on 2026-09-25 (e.g. `.../models/gemini/3-8-flash`, `.../gemini/3-1-pro`,
/// `.../partner-models/claude/opus-5-5`, `.../claude/opus-4-8`):
/// - Gemini 3.x GA models (3.8 / 3.7 / 3.6 Flash, 3.5 Flash-Lite, 3.1
///   Flash-Lite) are served at `global` plus the `us` / `eu` multi-regions;
///   3.5 Flash adds some non-US regions but neither us-central1 nor us-east5.
/// - The Gemini 3 previews (3.1 Pro Preview, 3 Flash Preview) are `global`
///   ONLY.
/// - Claude Opus 4.7, Opus 4.8 and every Claude 5.x model (Sonnet 5, Opus 5,
///   Opus 5.5, Fable 5, Fable 5.1) are served at `global` plus `us` / `eu`
///   only (`.../claude/opus-4-7` lists no single region).
/// - Older Claude (Opus 4.6 / Sonnet 4.6 and below: us-east5, europe-west1,
///   global) and Gemini 2.x are regional, which is what this provider always
///   assumed.
///
/// So with the provider's default `GCP_LOCATION` (us-central1) none of the
/// first three groups is reachable at the configured region; a request for
/// one is routed by [`GcpVertexAIModel::preferred_location`] instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelAvailability {
    /// Served in single regions: the configured `GCP_LOCATION` is honoured.
    Regional,
    /// Served at `global` and the `us` / `eu` multi-regions, in no single
    /// region the provider would pick.
    MultiRegion,
    /// Served at `global` and nowhere else.
    GlobalOnly,
}

/// The first number in a model id — `gemini-3.8-flash` → 3,
/// `claude-opus-5-5` → 5, `claude-sonnet-4-5@20250929` → 4.
fn major_version(model_name: &str) -> Option<u32> {
    model_name
        .split(['-', '.', '@'])
        .find_map(|segment| segment.parse::<u32>().ok())
}

/// Represents errors that can occur during model operations.
///
/// This enum encompasses various error conditions that might arise when working
/// with GCP Vertex AI models, including unsupported models, invalid requests,
/// and unsupported locations.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    /// Error when an unsupported Vertex AI model is specified
    #[error("Unsupported Vertex AI model: {0}")]
    UnsupportedModel(String),
    /// Error when the request structure is invalid
    #[error("Invalid request structure: {0}")]
    InvalidRequest(String),
    /// Error when an unsupported GCP location is specified
    #[error("Unsupported GCP location: {0}")]
    UnsupportedLocation(String),
}

/// Default model for GCP Vertex AI: Gemini 3.8 Flash, GA on Vertex since
/// 2026-09-02 and the first featured model on its Google-models page. Like
/// every Gemini 3.x model it is not served in us-central1, so it reaches the
/// global endpoint through [`GcpVertexAIModel::preferred_location`].
pub const DEFAULT_MODEL: &str = "gemini-3.8-flash";

// Verified against Anthropic's Vertex model-id table and the Vertex model and
// lifecycle pages (2026-09-25). Claude 4.6+ models have no @date suffix on
// Vertex; the 4.5 generation keeps it. Removed on 2026-09-25: gemini-3-pro
// (never a Vertex id — only gemini-3-pro-preview was ever published, and it is
// gone), gemini-3.1-pro (the id is gemini-3.1-pro-preview), and gemini-2.5-pro
// / -flash / -flash-lite (Vertex retires all three on 2026-10-20). Removed
// earlier: claude-opus-4@20250514 / claude-sonnet-4@20250514 (retired Jun 15,
// 2026), claude-opus-4-1@20250805, claude-3-5-haiku / claude-3-haiku
// (retired), gemini-2.0-flash(-lite) (discontinued Jun 1, 2026).
// Not listed: claude-mythos-5 / -5-1 (limited availability on Vertex).
//
// DEFAULT_MODEL comes first: the UI auto-selects `known_models[0]` when a
// user switches providers (SwitchModelModal), not the declared default, and a
// Gemini model works in any project while a Claude model must first be
// enabled in the project's Model Garden.
pub const KNOWN_MODELS: &[&str] = &[
    DEFAULT_MODEL,
    "gemini-3.7-flash",
    "gemini-3.6-flash",
    "gemini-3.5-flash",
    "gemini-3.5-flash-lite",
    "gemini-3.1-pro-preview",
    "gemini-3.1-flash-lite",
    "gemini-3-flash-preview",
    // Claude Opus 5.5 (GA 2026-09-22) and Fable 5.1 (GA 2026-09-01) bind
    // their thinking blocks to the conversation; `create_anthropic_request`
    // strips replayed ones.
    "claude-opus-5-5",
    "claude-fable-5-1",
    "claude-opus-5",
    "claude-fable-5",
    "claude-sonnet-5",
    "claude-opus-4-8",
    "claude-opus-4-7",
    "claude-opus-4-6",
    "claude-sonnet-4-6",
    "claude-opus-4-5@20251101",
    "claude-sonnet-4-5@20250929",
    "claude-haiku-4-5@20251001",
];

/// Represents available GCP Vertex AI models for biorouter.
///
/// This enum encompasses different model families that are supported
/// in the GCP Vertex AI platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GcpVertexAIModel {
    /// Claude model family
    Claude(String),
    /// Gemini model family
    Gemini(String),
    /// MaaS (Model as a Service) models from Model Garden
    /// Contains (publisher, full_model_name)
    MaaS(String, String),
}

impl fmt::Display for GcpVertexAIModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Claude(name) => write!(f, "{name}"),
            Self::Gemini(name) => write!(f, "{name}"),
            Self::MaaS(_, name) => write!(f, "{name}"),
        }
    }
}

impl GcpVertexAIModel {
    /// Where Vertex serves this model; see [`ModelAvailability`].
    ///
    /// Decided by family and major version rather than a list of ids, so a
    /// model typed in through "Enter a model not listed" lands on the same
    /// side as its siblings: Gemini 3+ and Claude 5+ are multi-region (their
    /// `-preview` Gemini ids global-only), Claude Opus 4.7 and 4.8 are the
    /// 4.x models served only at global and multi-region, and everything
    /// older stays regional.
    pub fn availability(&self) -> ModelAvailability {
        /// Claude 4.x ids Vertex serves at global and multi-region only.
        const MULTI_REGION_CLAUDE_4: &[&str] = &["claude-opus-4-7", "claude-opus-4-8"];
        match self {
            Self::Gemini(name) => match major_version(name) {
                Some(major) if major >= 3 => {
                    if name.contains("-preview") {
                        ModelAvailability::GlobalOnly
                    } else {
                        ModelAvailability::MultiRegion
                    }
                }
                _ => ModelAvailability::Regional,
            },
            Self::Claude(name) => {
                let multi_region = MULTI_REGION_CLAUDE_4
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
                    || major_version(name).is_some_and(|major| major >= 5);
                if multi_region {
                    ModelAvailability::MultiRegion
                } else {
                    ModelAvailability::Regional
                }
            }
            Self::MaaS(_, _) => ModelAvailability::Regional,
        }
    }

    /// Returns the location to fall back to when the first attempt fails.
    ///
    /// A regional model keeps its family's well-known region:
    /// - Claude models default to Ohio (us-east5)
    /// - Gemini models default to Iowa (us-central1)
    /// - MaaS models default to Iowa (us-central1)
    ///
    /// A model no single region serves falls back to the global endpoint.
    pub fn known_location(&self) -> GcpLocation {
        if self.availability() != ModelAvailability::Regional {
            return GcpLocation::Global;
        }
        match self {
            Self::Claude(_) => GcpLocation::Ohio,
            Self::Gemini(_) => GcpLocation::Iowa,
            Self::MaaS(_, _) => GcpLocation::Iowa,
        }
    }

    /// The location to send this model's request to first, given the
    /// configured `GCP_LOCATION`.
    ///
    /// A regional model goes where it is configured, as before. A model only
    /// served outside the single regions goes straight to the global endpoint
    /// instead of failing at the configured region first — which, with the
    /// default us-central1, is every Gemini 3.x model including the default,
    /// and every Claude 5.x model. A user who configured the `us` or `eu`
    /// multi-region keeps it for the models served there, so data stays in
    /// that geography (the provider's `route` gives such a request no
    /// fallback to global); global-only previews still have to go global. A
    /// configured single region that happens to serve one of these models
    /// (3.5 Flash is in a few non-US regions) is not consulted — setting
    /// `us` or `eu` is how a user keeps them in one geography.
    pub fn preferred_location(&self, configured: &str) -> String {
        let configured_is_multi_region = MULTI_REGION_LOCATIONS.contains(&configured);
        match self.availability() {
            ModelAvailability::Regional => configured.to_string(),
            ModelAvailability::MultiRegion
                if configured == GLOBAL_LOCATION || configured_is_multi_region =>
            {
                configured.to_string()
            }
            ModelAvailability::MultiRegion | ModelAvailability::GlobalOnly => {
                GLOBAL_LOCATION.to_string()
            }
        }
    }
}

impl TryFrom<&str> for GcpVertexAIModel {
    type Error = ModelError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        if s.starts_with("claude-") {
            Ok(Self::Claude(s.to_string()))
        } else if s.starts_with("gemini-") {
            Ok(Self::Gemini(s.to_string()))
        } else if s.ends_with("-maas") {
            let publisher = s
                .split('-')
                .next()
                .ok_or_else(|| ModelError::UnsupportedModel(s.to_string()))?
                .to_string();
            Ok(Self::MaaS(publisher, s.to_string()))
        } else {
            Err(ModelError::UnsupportedModel(s.to_string()))
        }
    }
}

/// Holds context information for a model request since the Vertex AI platform
/// supports multiple model families.
///
/// This structure maintains information about the model being used
/// and provides utility methods for handling model-specific operations.
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// The GCP Vertex AI model being used
    pub model: GcpVertexAIModel,
}

impl RequestContext {
    /// Creates a new RequestContext from a model ID string.
    ///
    /// # Arguments
    /// * `model_id` - The string identifier of the model
    ///
    /// # Returns
    /// * `Result<Self>` - A new RequestContext if the model ID is valid
    pub fn new(model_id: &str) -> Result<Self> {
        Ok(Self {
            model: GcpVertexAIModel::try_from(model_id)
                .with_context(|| format!("Failed to parse model ID: {model_id}"))?,
        })
    }

    /// Returns the provider associated with the model.
    pub fn provider(&self) -> ModelProvider {
        match &self.model {
            GcpVertexAIModel::Claude(_) => ModelProvider::Anthropic,
            GcpVertexAIModel::Gemini(_) => ModelProvider::Google,
            GcpVertexAIModel::MaaS(publisher, _) => ModelProvider::MaaS(publisher.clone()),
        }
    }
}

/// Represents available model providers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelProvider {
    /// Anthropic provider (Claude models)
    Anthropic,
    /// Google provider (Gemini models)
    Google,
    /// MaaS provider (Model as a Service from Model Garden)
    MaaS(String),
}

impl ModelProvider {
    /// Returns the string representation of the provider.
    pub fn as_str(&self) -> String {
        match self {
            Self::Anthropic => "anthropic".to_string(),
            Self::Google => "google".to_string(),
            Self::MaaS(publisher) => publisher.clone(),
        }
    }
}

/// Creates an Anthropic-specific Vertex AI request payload.
///
/// # Arguments
/// * `model_config` - Configuration for the model
/// * `system` - System prompt
/// * `messages` - Array of messages
/// * `tools` - Array of available tools
///
/// # Returns
/// * `Result<Value>` - JSON request payload for Anthropic API
fn create_anthropic_request(
    model_config: &ModelConfig,
    system: &str,
    messages: &[Message],
    tools: &[Tool],
) -> Result<Value> {
    // Opus 5.5 / Fable 5.1 bind each thinking block to the conversation
    // prefix, which BioRouter changes on every request (see
    // `anthropic::uses_preserved_thinking`), and a new Google Cloud project
    // gets a 400 for a replayed block whose prefix moved. The Claude API's
    // `drop_block` control is rejected on Vertex until Google enables it per
    // model (Anthropic platform-availability table, 2026-09-25), so the
    // replayed blocks are stripped here instead. Everything else replays as
    // before.
    let stripped;
    let messages = if anthropic::uses_preserved_thinking(&model_config.model_name) {
        stripped = anthropic::without_replayed_thinking(messages);
        stripped.as_slice()
    } else {
        messages
    };
    let mut request = anthropic::create_request(model_config, system, messages, tools)?;

    let obj = request
        .as_object_mut()
        .ok_or_else(|| ModelError::InvalidRequest("Request is not a JSON object".to_string()))?;

    // Note: We don't need to specify the model in the request body
    // The model is determined by the endpoint URL in GCP Vertex AI
    obj.remove("model");
    obj.insert(
        "anthropic_version".to_string(),
        Value::String("vertex-2023-10-16".to_string()),
    );

    Ok(request)
}

/// Creates a Gemini-specific Vertex AI request payload.
///
/// # Arguments
/// * `model_config` - Configuration for the model
/// * `system` - System prompt
/// * `messages` - Array of messages
/// * `tools` - Array of available tools
///
/// # Returns
/// * `Result<Value>` - JSON request payload for Google API
fn create_google_request(
    model_config: &ModelConfig,
    system: &str,
    messages: &[Message],
    tools: &[Tool],
) -> Result<Value> {
    google::create_request(model_config, system, messages, tools)
}

/// Creates a provider-specific request payload and context.
///
/// # Arguments
/// * `model_config` - Configuration for the model
/// * `system` - System prompt
/// * `messages` - Array of messages
/// * `tools` - Array of available tools
///
/// # Returns
/// * `Result<(Value, RequestContext)>` - Tuple of request payload and context
pub fn create_request(
    model_config: &ModelConfig,
    system: &str,
    messages: &[Message],
    tools: &[Tool],
) -> Result<(Value, RequestContext)> {
    let context = RequestContext::new(&model_config.model_name)?;

    let request = match &context.model {
        GcpVertexAIModel::Claude(_) => {
            create_anthropic_request(model_config, system, messages, tools)?
        }
        GcpVertexAIModel::Gemini(_) => {
            create_google_request(model_config, system, messages, tools)?
        }
        GcpVertexAIModel::MaaS(_, _) => {
            // TODO: Branch on publisher for format selection once we know which
            // MaaS providers use which formats (e.g., OpenAI vs Google format)
            // For now, default to Google format since most use generateContent endpoint
            create_google_request(model_config, system, messages, tools)?
        }
    };

    Ok((request, context))
}

/// Converts a provider response to a Message.
///
/// # Arguments
/// * `response` - The raw response from the provider
/// * `request_context` - Context information about the request
///
/// # Returns
/// * `Result<Message>` - Converted message
pub fn response_to_message(response: Value, request_context: RequestContext) -> Result<Message> {
    match request_context.provider() {
        ModelProvider::Anthropic => anthropic::response_to_message(&response),
        ModelProvider::Google => google::response_to_message(response),
        ModelProvider::MaaS(_) => google::response_to_message(response),
    }
}

/// Extracts token usage information from the response data.
///
/// # Arguments
/// * `data` - The response data containing usage information
/// * `request_context` - Context information about the request
///
/// # Returns
/// * `Result<Usage>` - Usage statistics
pub fn get_usage(data: &Value, request_context: &RequestContext) -> Result<Usage> {
    match request_context.provider() {
        ModelProvider::Anthropic => anthropic::get_usage(data),
        ModelProvider::Google => google::get_usage(data),
        ModelProvider::MaaS(_) => google::get_usage(data),
    }
}

pub fn response_to_streaming_message<S>(
    stream: S,
    request_context: &RequestContext,
) -> StreamingMessageStream
where
    S: futures::Stream<Item = anyhow::Result<String>> + Unpin + Send + 'static,
{
    match request_context.provider() {
        ModelProvider::Anthropic => Box::pin(anthropic::response_to_streaming_message(stream)),
        ModelProvider::Google | ModelProvider::MaaS(_) => {
            Box::pin(google::response_to_streaming_message(stream))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn test_model_parsing() -> Result<()> {
        let claude = GcpVertexAIModel::try_from("claude-sonnet-4@20250514")?;
        assert!(matches!(claude, GcpVertexAIModel::Claude(_)));
        assert_eq!(claude.to_string(), "claude-sonnet-4@20250514");

        let gemini = GcpVertexAIModel::try_from("gemini-2.5-flash")?;
        assert!(matches!(gemini, GcpVertexAIModel::Gemini(_)));
        assert_eq!(gemini.to_string(), "gemini-2.5-flash");

        let maas = GcpVertexAIModel::try_from("qwen-maas")?;
        assert!(matches!(maas, GcpVertexAIModel::MaaS(_, _)));

        assert!(GcpVertexAIModel::try_from("unsupported-model").is_err());
        Ok(())
    }

    #[test]
    fn test_default_locations() -> Result<()> {
        let claude_model = GcpVertexAIModel::try_from("claude-sonnet-4@20250514")?;
        assert_eq!(claude_model.known_location(), GcpLocation::Ohio);

        let gemini_model = GcpVertexAIModel::try_from("gemini-2.5-flash")?;
        assert_eq!(gemini_model.known_location(), GcpLocation::Iowa);

        Ok(())
    }

    #[test]
    fn availability_follows_family_and_major_version() -> Result<()> {
        let cases = [
            // Gemini 3.x GA: global + multi-region, in no single region.
            ("gemini-3.8-flash", ModelAvailability::MultiRegion),
            ("gemini-3.5-flash", ModelAvailability::MultiRegion),
            ("gemini-3.5-flash-lite", ModelAvailability::MultiRegion),
            ("gemini-3.1-flash-lite", ModelAvailability::MultiRegion),
            // An unlisted future Gemini lands with its siblings.
            ("gemini-4.0-ultra", ModelAvailability::MultiRegion),
            // Gemini 3 previews: global only.
            ("gemini-3.1-pro-preview", ModelAvailability::GlobalOnly),
            ("gemini-3-flash-preview", ModelAvailability::GlobalOnly),
            // Claude 5.x and Opus 4.7 / 4.8: global + multi-region.
            ("claude-opus-5-5", ModelAvailability::MultiRegion),
            ("claude-fable-5-1", ModelAvailability::MultiRegion),
            ("claude-sonnet-5", ModelAvailability::MultiRegion),
            ("claude-opus-4-8", ModelAvailability::MultiRegion),
            ("claude-opus-4-7", ModelAvailability::MultiRegion),
            // Regional, exactly as the provider always assumed.
            ("claude-opus-4-6", ModelAvailability::Regional),
            ("claude-sonnet-4-6", ModelAvailability::Regional),
            ("claude-sonnet-4-5@20250929", ModelAvailability::Regional),
            ("claude-sonnet-4@20250514", ModelAvailability::Regional),
            ("claude-future-version", ModelAvailability::Regional),
            ("gemini-2.5-flash", ModelAvailability::Regional),
            ("qwen-maas", ModelAvailability::Regional),
        ];
        for (model, expected) in cases {
            assert_eq!(
                GcpVertexAIModel::try_from(model)?.availability(),
                expected,
                "{model}"
            );
        }
        Ok(())
    }

    #[test]
    fn non_regional_models_fall_back_to_global() -> Result<()> {
        assert_eq!(
            GcpVertexAIModel::try_from(DEFAULT_MODEL)?.known_location(),
            GcpLocation::Global
        );
        assert_eq!(
            GcpVertexAIModel::try_from("claude-opus-5-5")?.known_location(),
            GcpLocation::Global
        );
        assert_eq!(GcpLocation::Global.to_string(), "global");
        assert_eq!(GcpLocation::try_from("global")?, GcpLocation::Global);
        Ok(())
    }

    /// Every advertised model is reachable with the default `GCP_LOCATION`:
    /// either it is regional (so us-central1 or its family's region serves
    /// it), or it is routed away from us-central1 before the request is made.
    ///
    /// The classification is checked against the vendor's "Supported regions"
    /// tables (2026-09-25), one row per advertised id — deriving it from
    /// `availability()` itself would only restate the code. Adding a model to
    /// KNOWN_MODELS means looking up its regions and adding its row here.
    #[test]
    fn every_listed_model_has_a_route_from_the_default_location() -> Result<()> {
        use ModelAvailability::{GlobalOnly, MultiRegion, Regional};
        let vendor_regions = [
            ("gemini-3.8-flash", MultiRegion),
            ("gemini-3.7-flash", MultiRegion),
            ("gemini-3.6-flash", MultiRegion),
            ("gemini-3.5-flash", MultiRegion),
            ("gemini-3.5-flash-lite", MultiRegion),
            ("gemini-3.1-pro-preview", GlobalOnly),
            ("gemini-3.1-flash-lite", MultiRegion),
            ("gemini-3-flash-preview", GlobalOnly),
            ("claude-opus-5-5", MultiRegion),
            ("claude-fable-5-1", MultiRegion),
            ("claude-opus-5", MultiRegion),
            ("claude-fable-5", MultiRegion),
            ("claude-sonnet-5", MultiRegion),
            ("claude-opus-4-8", MultiRegion),
            ("claude-opus-4-7", MultiRegion),
            ("claude-opus-4-6", Regional),
            ("claude-sonnet-4-6", Regional),
            ("claude-opus-4-5@20251101", Regional),
            ("claude-sonnet-4-5@20250929", Regional),
            ("claude-haiku-4-5@20251001", Regional),
        ];
        let mut listed: Vec<&str> = KNOWN_MODELS.to_vec();
        let mut tabled: Vec<&str> = vendor_regions.iter().map(|(model, _)| *model).collect();
        listed.sort_unstable();
        tabled.sort_unstable();
        assert_eq!(listed, tabled, "every advertised id needs a vendor row");
        for (model, expected) in vendor_regions {
            assert_eq!(
                GcpVertexAIModel::try_from(model)?.availability(),
                expected,
                "{model}"
            );
        }

        let default_location = GcpLocation::Iowa.to_string();
        for model in KNOWN_MODELS {
            let parsed = GcpVertexAIModel::try_from(*model)?;
            let preferred = parsed.preferred_location(&default_location);
            match parsed.availability() {
                ModelAvailability::Regional => assert_eq!(preferred, default_location, "{model}"),
                _ => assert_eq!(preferred, GLOBAL_LOCATION, "{model}"),
            }
        }
        Ok(())
    }

    // Opus 5.5 on Vertex: replayed thinking is stripped (no drop_block on
    // Vertex yet), and no binding control or beta leaks into the body. Opus
    // 4.8 replays its thinking exactly as before.
    #[test]
    fn claude_requests_strip_thinking_only_for_preserved_thinking_models() -> Result<()> {
        let history = [
            Message::user().with_text("first"),
            Message::assistant()
                .with_thinking("", "sig-1")
                .with_text("answer"),
            Message::user().with_text("second"),
        ];

        let config = ModelConfig::new_or_fail("claude-opus-5-5");
        let (request, _) = create_request(&config, "system", &history, &[])?;
        let rendered = request.to_string();
        assert!(!rendered.contains("sig-1"), "{rendered}");
        assert!(!rendered.contains("block_binding"), "{rendered}");
        assert_eq!(request["anthropic_version"], "vertex-2023-10-16");
        assert_eq!(request["messages"].as_array().unwrap().len(), 3);

        let config = ModelConfig::new_or_fail("claude-opus-4-8");
        let (request, _) = create_request(&config, "system", &history, &[])?;
        assert!(request.to_string().contains("sig-1"));
        Ok(())
    }

    #[test]
    fn test_unknown_model_parsing() -> Result<()> {
        let model = GcpVertexAIModel::try_from("claude-future-version")?;
        assert!(matches!(model, GcpVertexAIModel::Claude(_)));
        assert_eq!(model.to_string(), "claude-future-version");

        let model = GcpVertexAIModel::try_from("gemini-4.0-ultra")?;
        assert!(matches!(model, GcpVertexAIModel::Gemini(_)));
        assert_eq!(model.to_string(), "gemini-4.0-ultra");

        Ok(())
    }
}
