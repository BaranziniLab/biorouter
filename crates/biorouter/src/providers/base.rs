use anyhow::Result;
use futures::Stream;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

use super::canonical::{map_to_canonical_model, CanonicalModelRegistry};
use super::errors::ProviderError;
use super::retry::RetryConfig;
use crate::config::base::ConfigValue;
use crate::conversation::message::Message;
use crate::conversation::Conversation;
use crate::model::ModelConfig;
use crate::privacy::affiliation::ModelAffiliation;
use crate::privacy::ProviderTier;
use crate::utils::safe_truncate;
use rmcp::model::Tool;
use utoipa::ToSchema;

use once_cell::sync::Lazy;
use std::ops::{Add, AddAssign};
use std::pin::Pin;
use std::sync::Mutex;

/// A global store for the current model being used, we use this as when a provider returns, it tells us the real model, not an alias
pub static CURRENT_MODEL: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

/// Set the current model in the global store
pub fn set_current_model(model: &str) {
    if let Ok(mut current_model) = CURRENT_MODEL.lock() {
        *current_model = Some(model.to_string());
    }
}

/// Get the current model from the global store, the real model, not an alias
pub fn get_current_model() -> Option<String> {
    CURRENT_MODEL.lock().ok().and_then(|model| model.clone())
}

pub static MSG_COUNT_FOR_SESSION_NAME_GENERATION: usize = 3;

/// Information about a model's capabilities
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct ModelInfo {
    /// The name of the model
    pub name: String,
    /// The maximum context length this model supports
    pub context_limit: usize,
    /// Cost per token for input (optional)
    pub input_token_cost: Option<f64>,
    /// Cost per token for output (optional)
    pub output_token_cost: Option<f64>,
    /// Currency for the costs (default: "$")
    pub currency: Option<String>,
    /// Whether this model supports cache control
    pub supports_cache_control: Option<bool>,
    /// Whether this model accepts image inputs (multimodal vision)
    #[serde(default)]
    pub supports_vision: Option<bool>,
    /// MIME types that can be sent as structured model input blocks.
    #[serde(default)]
    pub supported_input_mime_types: Option<Vec<String>>,
}

impl ModelInfo {
    /// Create a new ModelInfo with just name and context limit
    pub fn new(name: impl Into<String>, context_limit: usize) -> Self {
        Self {
            name: name.into(),
            context_limit,
            input_token_cost: None,
            output_token_cost: None,
            currency: None,
            supports_cache_control: None,
            supports_vision: None,
            supported_input_mime_types: None,
        }
    }

    /// Create a new ModelInfo with cost information (per token)
    pub fn with_cost(
        name: impl Into<String>,
        context_limit: usize,
        input_cost: f64,
        output_cost: f64,
    ) -> Self {
        Self {
            name: name.into(),
            context_limit,
            input_token_cost: Some(input_cost),
            output_token_cost: Some(output_cost),
            currency: Some("$".to_string()),
            supports_cache_control: None,
            supports_vision: None,
            supported_input_mime_types: None,
        }
    }

    /// Mark this model as supporting image inputs (multimodal vision).
    /// Mark the model as **known** not to take images.
    ///
    /// Distinct from leaving `supports_vision` as `None`, which says only that
    /// nobody has looked. A provider that can enumerate its catalog knows the
    /// answer and should say so: Codex's `model/list` reports `inputModalities`
    /// per model, and `gpt-5.3-codex-spark` is `["text"]`. Recording that as
    /// "unknown" throws away a fact the provider was told.
    pub fn without_vision(mut self) -> Self {
        self.supports_vision = Some(false);
        self
    }

    pub fn with_vision(mut self) -> Self {
        self.supports_vision = Some(true);
        if self.supported_input_mime_types.is_none() {
            self.supported_input_mime_types = Some(
                [
                    "image/png",
                    "image/jpeg",
                    "image/jpg",
                    "image/gif",
                    "image/webp",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
            );
        }
        self
    }

    pub fn with_supported_input_mime_types<I, S>(mut self, mime_types: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mime_types: Vec<String> = mime_types.into_iter().map(Into::into).collect();
        if mime_types
            .iter()
            .any(|mime_type| mime_type.starts_with("image/"))
        {
            self.supports_vision = Some(true);
        }
        self.supported_input_mime_types = Some(mime_types);
        self
    }

    pub fn with_png_jpeg_image_inputs(self) -> Self {
        self.with_supported_input_mime_types(["image/png", "image/jpeg", "image/jpg"])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum ProviderType {
    Preferred,
    Builtin,
    Declarative,
    Custom,
}

/// Metadata about a provider's configuration requirements and capabilities
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProviderMetadata {
    /// The unique identifier for this provider
    pub name: String,
    /// Display name for the provider in UIs
    pub display_name: String,
    /// Description of the provider's capabilities
    pub description: String,
    /// The default/recommended model for this provider
    pub default_model: String,
    /// A list of currently known models with their capabilities
    pub known_models: Vec<ModelInfo>,
    /// Link to the docs where models can be found
    pub model_doc_link: String,
    /// Required configuration keys
    pub config_keys: Vec<ConfigKey>,
    /// Whether this provider allows entering model names not in the fetched list
    #[serde(default)]
    pub allows_unlisted_models: bool,
    /// Whether models from this provider may be bound to a private session.
    /// Serialize + ToSchema, so it reaches every UI surface through
    /// `just generate-openapi` -> `npm run generate-api`.
    ///
    /// This is the *type-level* claim, computed from the endpoint this provider
    /// ships with; an instance that resolved somewhere else reports its own
    /// tier from [`Provider::tier`], which is the only value the enforcement
    /// path reads.
    ///
    /// ⚠ The two can disagree, and this one can only ever be the *more*
    /// generous of the pair. `GET /config/providers` serves this field
    /// verbatim, so an `ollama` re-pointed off the machine by `OLLAMA_HOST`
    /// still ships `Private` here while its instance resolves `Public`. That is
    /// harmless for the settings grid, which is only choosing a section
    /// heading, and it is **not** harmless for a privacy badge: do not hang one
    /// on this field. A badge has to read the tier of the instance actually
    /// bound to the session, which means plumbing [`Provider::tier`] out to the
    /// UI first — it would otherwise read Private in exactly the demotion case
    /// the tier exists to catch.
    #[serde(default)]
    pub tier: ProviderTier,
    /// Whether this provider's inference runs on the user's own machine — a
    /// bundled or self-hosted server — rather than on a remote service.
    ///
    /// Display only: it is what splits the private tier into the settings
    /// grid's "Local Models" and "Institutional Models" sections. It is **not**
    /// the privacy tier, and neither field is derivable from the other: a
    /// self-hosted server pointed off the machine is still `runs_locally` by
    /// type and Public by instance.
    #[serde(default)]
    pub runs_locally: bool,
    /// The institution(s) whose gateway this provider *ships* pointed at — the
    /// **type-level** institution claim, exactly as [`Self::tier`] is the
    /// type-level tier claim, and declared by the same builder pattern
    /// ([`Self::with_institution`]) in the provider's own module.
    ///
    /// ## Why this exists beside `ProviderDetails::affiliation`
    ///
    /// `ProviderDetails::affiliation` is instance-resolved and strictly better,
    /// and it is what any surface should prefer. But `GET /config/providers`
    /// resolves it **only for a configured provider** — an unconfigured one has
    /// no keys, cannot be constructed, and must not have every provider module's
    /// constructor run on a plain GET (see `resolve_provider_axes`). So on a
    /// machine where nothing is set up yet — first-run onboarding, the one place
    /// a user most needs to be told which gateway belongs to their institution —
    /// every row's `affiliation` is `null`. This field is the answer for exactly
    /// that gap: it lets a catalog head a group "UCSF" from the daemon's own
    /// data rather than from a name-keyed table in the renderer.
    ///
    /// ⚠ **Not a badge, and not an enforcement input.** Same warning as
    /// [`Self::tier`]'s, and for the same reason: this is what the provider
    /// *ships* pointed at, so a Versa instance repointed at another host still
    /// carries `ucsf` here while [`Provider::affiliation`] correctly reports
    /// `None`. It may only ever be read as a *fallback for a row the daemon did
    /// not resolve*, never in preference to an instance answer, and never by a
    /// gate. `privacy::affiliation` reads [`Provider::affiliation`] and nothing
    /// else.
    ///
    /// Empty for every provider that ships pointed at no institution, which is
    /// almost all of them.
    #[serde(default)]
    pub institutions: Vec<AffiliationInstitution>,
}

impl ProviderMetadata {
    pub fn new(
        name: &str,
        display_name: &str,
        description: &str,
        default_model: &str,
        model_names: Vec<&str>,
        model_doc_link: &str,
        config_keys: Vec<ConfigKey>,
    ) -> Self {
        Self {
            name: name.to_string(),
            display_name: display_name.to_string(),
            description: description.to_string(),
            default_model: default_model.to_string(),
            known_models: model_names
                .iter()
                .map(|&name| ModelInfo {
                    name: name.to_string(),
                    context_limit: ModelConfig::new_or_fail(name).context_limit(),
                    input_token_cost: None,
                    output_token_cost: None,
                    currency: None,
                    supports_cache_control: None,
                    supports_vision: None,
                    supported_input_mime_types: None,
                })
                .collect(),
            model_doc_link: model_doc_link.to_string(),
            config_keys,
            allows_unlisted_models: false,
            tier: ProviderTier::default(),
            runs_locally: false,
            institutions: Vec::new(),
        }
    }

    pub fn with_models(
        name: &str,
        display_name: &str,
        description: &str,
        default_model: &str,
        models: Vec<ModelInfo>,
        model_doc_link: &str,
        config_keys: Vec<ConfigKey>,
    ) -> Self {
        Self {
            name: name.to_string(),
            display_name: display_name.to_string(),
            description: description.to_string(),
            default_model: default_model.to_string(),
            known_models: models,
            model_doc_link: model_doc_link.to_string(),
            config_keys,
            allows_unlisted_models: false,
            tier: ProviderTier::default(),
            runs_locally: false,
            institutions: Vec::new(),
        }
    }

    pub fn empty() -> Self {
        Self {
            name: "".to_string(),
            display_name: "".to_string(),
            description: "".to_string(),
            default_model: "".to_string(),
            known_models: vec![],
            model_doc_link: "".to_string(),
            config_keys: vec![],
            allows_unlisted_models: false,
            tier: ProviderTier::default(),
            runs_locally: false,
            institutions: Vec::new(),
        }
    }

    /// Set allows_unlisted_models flag (builder pattern)
    pub fn with_unlisted_models(mut self) -> Self {
        self.allows_unlisted_models = true;
        self
    }

    /// Declare the tier this provider ships at. Each provider states its own,
    /// in its own module — there is no central list of private providers, so
    /// there is nothing for a new provider to be forgotten from.
    pub fn with_tier(mut self, tier: ProviderTier) -> Self {
        self.tier = tier;
        self
    }

    /// Declare that this provider's inference runs on the user's machine.
    pub fn with_local_compute(mut self) -> Self {
        self.runs_locally = true;
        self
    }

    /// Declare the institution whose gateway this provider ships pointed at —
    /// the type-level half of DR-26's third axis. See [`Self::institutions`] for
    /// what it may and may not be read for.
    ///
    /// The display name is looked up in the same institution registry every
    /// warning and badge reads, so a catalog printing this name and a warning
    /// printing that name cannot drift. An institution the registry does not
    /// publish keeps its bare id, exactly as `privacy::affiliation::label` does
    /// — a real constraint must never disappear from a surface because nobody
    /// wrote it a pretty name.
    pub fn with_institution(mut self, id: &str) -> Self {
        let interned = crate::privacy::affiliation::InstitutionId::new(id);
        self.institutions.push(AffiliationInstitution {
            id: interned.as_str().to_string(),
            display_name: crate::privacy::affiliation::institution_display_name(interned)
                .map(str::to_string),
        });
        self
    }
}

/// One institution, as a surface that has to *print* it needs it (issue #56,
/// DR-26).
///
/// Both halves travel, and neither is derivable from the other. The `id` is the
/// normalised slug — what `registry.json` publishes, what `baam.html`'s
/// `data-affiliation` carries, what every cross-affiliation warning prints and
/// what a user quotes in a support conversation. The `display_name` is what the
/// registry publishes for it, and is **absent** for an institution the registry
/// does not know.
///
/// ⚠ **An absent display name is not an error and must never render as
/// nothing.** Task 47 rules that an affiliation naming an unpublished
/// institution is a *mismatch* whose raw id is surfaced; dropping the row
/// because it has no pretty name would make a real constraint disappear from the
/// one surface a user can see it on. This mirrors `privacy::affiliation::label`,
/// which falls back to the bare id for exactly that reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AffiliationInstitution {
    /// The normalised slug (`ucsf`).
    pub id: String,
    /// The registry's published display name (`UCSF`), or `None`.
    #[serde(default)]
    pub display_name: Option<String>,
}

/// Which of DR-26's three model-side affiliations this is.
///
/// ⚠ **There is no `Public` variant, and that is the whole shape of the type.**
/// A public model's affiliation is not a value here, it is the *absence* of a
/// [`ProviderAffiliation`] — `Option::None` — exactly as
/// [`ModelAffiliation`] has no public variant. Giving "public" a seat would
/// invite a renderer to draw a chip for it, and a chip that says "no
/// institution" on a public model reads as a *constraint* on a model that has
/// none of the private tier's protections at all.
///
/// A closed enum rather than a free string so a renderer can switch on it
/// totally: a fourth affiliation must be a compile error at the badge, not a
/// silently-unhandled branch that renders as the safest-looking of the three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAffiliationKind {
    /// [`ModelAffiliation::Local`] — inference runs on this machine.
    ///
    /// ⚠ **The MOST permissive affiliation, not the narrowest.** It reaches
    /// every private extension, because no transfer occurs at all and there is
    /// therefore nothing for an agreement to govern
    /// (`privacy::affiliation::compatible`'s first arm, which returns *before*
    /// any comparison). A surface that renders this beside an institution's name
    /// and lets it read as "even more restricted than UCSF" has inverted the
    /// axis — see `ProviderAffiliation`'s own note.
    Local,
    /// [`ModelAffiliation::Institutions`] — covered by the named institutions'
    /// agreements. [`ProviderAffiliation::institutions`] is non-empty exactly
    /// for this variant.
    Institutions,
    /// A **private** model that states no institution at all.
    ///
    /// ⚠ **Meaningful, not missing.** DR-26 gives "no affiliation" one meaning —
    /// a public model — so a Private tier with no affiliation is a combination
    /// the vocabulary says cannot exist, and it is nevertheless reachable (a
    /// provider that overrides `tier()` and forgets `affiliation()` inherits the
    /// trait default). `privacy::affiliation::unstated_model` treats such a model
    /// as mismatching **every** named institution, so it clears only extensions
    /// with no institutional constraint. Rendering it as an empty affiliation, or
    /// as no affiliation at all, would show the least-reaching private model as
    /// the least-constrained one.
    Unstated,
}

/// DR-26's third axis for one provider **instance**, in the shape a UI can
/// render (issue #56).
///
/// ⚠ **Resolved from the instance, never from a name.** [`Self::of`] is the only
/// constructor a caller should reach for, and it reads
/// [`Provider::tier`] and [`Provider::affiliation`] off one `&dyn Provider` —
/// the same two instance methods every gate reads. A `versa_* => ucsf` table
/// would keep claiming the institution for a Versa module repointed at another
/// host, which `tier()` has already demoted to Public: a private-looking
/// institution chip on a public flow. Both Versa providers derive `ucsf` from
/// the same gateway-host check that decides their tier
/// (`providers::ucsf_gateway_affiliation`), so a repointed instance loses
/// Private and `ucsf` **together**, and this type is what carries that pairing
/// out to the UI.
///
/// ⚠ **`Option<ProviderAffiliation>::None` is "public — no affiliation at all",
/// and it is a fourth state distinct from all three
/// [`ProviderAffiliationKind`]s.** See [`Self::resolved`] for the table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProviderAffiliation {
    /// Which affiliation this is.
    pub kind: ProviderAffiliationKind,
    /// The covering institutions, ascending by id. Non-empty exactly for
    /// [`ProviderAffiliationKind::Institutions`], and **never** empty there:
    /// `InstitutionSet` cannot be constructed empty, for the reason its own doc
    /// records (an empty model set is a subset of every allowlist and so reaches
    /// every private extension).
    #[serde(default)]
    pub institutions: Vec<AffiliationInstitution>,
}

impl ProviderAffiliation {
    /// The one mapping from the two instance reads to what a surface renders.
    ///
    /// | `tier()` | `affiliation()` | result |
    /// |---|---|---|
    /// | Public | anything | `None` — no affiliation to show |
    /// | Private | `Some(Local)` | `Local` |
    /// | Private | `Some(Institutions(M))` | `Institutions(M)` |
    /// | Private | `None` | `Unstated` |
    ///
    /// ⚠ **The tier is asked FIRST, and that ordering is the third of the four
    /// states.** It mirrors `privacy::affiliation::gate_cross_affiliation`, whose
    /// first line returns `None` for a non-private model tier before it looks at
    /// affiliation at all — so a public model has no third axis, in the gate and
    /// on the badge alike. Asking `affiliation()` first would collapse Public and
    /// `Unstated` into one rendering, and those are opposite facts: a public
    /// model is one the tier gates already keep away from private data, while an
    /// unstated private model is one that *may* reach unconstrained private
    /// extensions and nothing else.
    pub fn resolved(tier: ProviderTier, affiliation: Option<ModelAffiliation>) -> Option<Self> {
        if !tier.is_private() {
            // Public. Not `Unstated`, and not an empty institution list — see
            // `ProviderAffiliationKind`'s own doc for why there is deliberately
            // no variant to put here.
            return None;
        }
        Some(match affiliation {
            Some(ModelAffiliation::Local) => Self {
                kind: ProviderAffiliationKind::Local,
                institutions: Vec::new(),
            },
            Some(ModelAffiliation::Institutions(set)) => Self {
                kind: ProviderAffiliationKind::Institutions,
                // `InstitutionSet::iter` is ascending, so the rendered order is
                // stable across calls rather than a hash order that reshuffles
                // the chip between two renders of the same model.
                institutions: set
                    .iter()
                    .map(|id| AffiliationInstitution {
                        id: id.as_str().to_string(),
                        display_name: crate::privacy::affiliation::institution_display_name(id)
                            .map(str::to_string),
                    })
                    .collect(),
            },
            // Private with nothing stated. The arm
            // `privacy::affiliation::unstated_model` exists for, surfaced rather
            // than smoothed over.
            None => Self {
                kind: ProviderAffiliationKind::Unstated,
                institutions: Vec::new(),
            },
        })
    }

    /// [`Self::resolved`] taken off one live provider — **the** spelling every
    /// caller should use.
    ///
    /// Both reads come off the same `&dyn Provider`, in one place, so a surface
    /// cannot pair one instance's tier with another instance's affiliation. That
    /// is the same collapsing `privacy::CallCapability` performs for the gates,
    /// and for the same reason: the two axes are decided by one endpoint
    /// resolution and must be sampled together.
    pub fn of(provider: &dyn Provider) -> Option<Self> {
        Self::resolved(provider.tier(), provider.affiliation())
    }
}

/// Configuration key metadata for provider setup
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConfigKey {
    /// The name of the configuration key (e.g., "API_KEY")
    pub name: String,
    /// Whether this key is required for the provider to function
    pub required: bool,
    /// Whether this key should be stored securely (e.g., in keychain)
    pub secret: bool,
    /// Optional default value for the key
    pub default: Option<String>,
    /// Whether this key should be configured using OAuth device code flow
    /// When true, the provider's configure_oauth() method will be called instead of prompting for manual input
    pub oauth_flow: bool,
}

impl ConfigKey {
    /// Create a new ConfigKey
    pub fn new(name: &str, required: bool, secret: bool, default: Option<&str>) -> Self {
        Self {
            name: name.to_string(),
            required,
            secret,
            default: default.map(|s| s.to_string()),
            oauth_flow: false,
        }
    }

    pub fn from_value_type<T: ConfigValue>(required: bool, secret: bool) -> Self {
        Self {
            name: T::KEY.to_string(),
            required,
            secret,
            default: Some(T::DEFAULT.to_string()),
            oauth_flow: false,
        }
    }

    /// Create a new ConfigKey that uses OAuth device code flow for configuration
    ///
    /// This is used for providers that support OAuth authentication instead of manual API key entry.
    /// When oauth_flow is true, the configuration system will call the provider's configure_oauth() method.
    pub fn new_oauth(name: &str, required: bool, secret: bool, default: Option<&str>) -> Self {
        Self {
            name: name.to_string(),
            required,
            secret,
            default: default.map(|s| s.to_string()),
            oauth_flow: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub model: String,
    /// Concrete provider that served this call. Wrapper providers set this to
    /// the selected child provider so accounting never guesses from the model.
    #[serde(default)]
    pub provider: Option<String>,
    pub usage: Usage,
    /// The provider's stop/finish reason for the response, when reported
    /// (OpenAI-compatible streaming `choices[].finish_reason`, e.g. `"stop"`,
    /// `"length"`, `"tool_calls"`). `None` when the provider does not surface it.
    /// Used by the agent loop to auto-continue a turn that was cut off by the
    /// output-length limit (`"length"`) rather than completed naturally.
    #[serde(default)]
    pub finish_reason: Option<String>,
}

impl ProviderUsage {
    pub fn new(model: String, usage: Usage) -> Self {
        Self {
            model,
            provider: None,
            usage,
            finish_reason: None,
        }
    }

    /// Ensures this ProviderUsage has token counts, estimating them if necessary
    pub async fn ensure_tokens(
        &mut self,
        system_prompt: &str,
        request_messages: &[Message],
        response: &Message,
        tools: &[Tool],
    ) -> Result<(), ProviderError> {
        crate::providers::usage_estimator::ensure_usage_tokens(
            self,
            system_prompt,
            request_messages,
            response,
            tools,
        )
        .await
        .map_err(|e| ProviderError::ExecutionError(format!("Failed to ensure usage tokens: {}", e)))
    }

    /// Combine this ProviderUsage with another, adding their token counts
    /// Uses the model from this ProviderUsage
    pub fn combine_with(&self, other: &ProviderUsage) -> ProviderUsage {
        ProviderUsage {
            model: self.model.clone(),
            provider: other.provider.clone().or_else(|| self.provider.clone()),
            usage: self.usage + other.usage,
            // Prefer the most recent finish_reason (the terminal chunk's).
            finish_reason: other
                .finish_reason
                .clone()
                .or_else(|| self.finish_reason.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, Copy)]
pub struct Usage {
    /// Fresh (non-cached) input/prompt tokens. INVARIANT: this EXCLUDES the two
    /// cache buckets below — the four token buckets (`input`, `output`,
    /// `cache_read`, `cache_creation`) are disjoint by construction, so
    /// [`Usage::billed_total`] is a plain sum that reconciles with vendor
    /// billing dashboards. Parsers whose provider reports a cache-inclusive
    /// input count subtract the cache out before storing it here (see the
    /// per-provider notes on each `get_usage`).
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    /// Full context/window occupancy for this turn, for the live gauge. Kept as
    /// each provider reports (or computes) it, which for cache-aware providers
    /// already includes the cache tokens — so `total_tokens` is NOT the same as
    /// `input + output`, and is NOT the billed number. Use [`Usage::billed_total`]
    /// for billing/reconciliation.
    pub total_tokens: Option<i32>,
    /// Input tokens served from the provider's prompt cache at a reduced rate.
    /// Additive: NOT included in `input_tokens`. `serde(default)` so token rows
    /// persisted before this field existed still deserialize (as `None`).
    #[serde(default)]
    pub cache_read_input_tokens: Option<i32>,
    /// Input tokens written to the provider's prompt cache (billed at a premium).
    /// Additive: NOT included in `input_tokens`. `None` for providers that do not
    /// distinguish a cache-write step (e.g. OpenAI auto-caching only reads).
    #[serde(default)]
    pub cache_creation_input_tokens: Option<i32>,
}

fn sum_optionals<T>(a: Option<T>, b: Option<T>) -> Option<T>
where
    T: Add<Output = T> + Default,
{
    match (a, b) {
        (Some(x), Some(y)) => Some(x + y),
        (Some(x), None) => Some(x + T::default()),
        (None, Some(y)) => Some(T::default() + y),
        (None, None) => None,
    }
}

impl Add for Usage {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        let mut combined = Self::new(
            sum_optionals(self.input_tokens, other.input_tokens),
            sum_optionals(self.output_tokens, other.output_tokens),
            sum_optionals(self.total_tokens, other.total_tokens),
        );
        combined.cache_read_input_tokens =
            sum_optionals(self.cache_read_input_tokens, other.cache_read_input_tokens);
        combined.cache_creation_input_tokens = sum_optionals(
            self.cache_creation_input_tokens,
            other.cache_creation_input_tokens,
        );
        combined
    }
}

impl AddAssign for Usage {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Usage {
    pub fn new(
        input_tokens: Option<i32>,
        output_tokens: Option<i32>,
        total_tokens: Option<i32>,
    ) -> Self {
        let calculated_total = if total_tokens.is_none() {
            match (input_tokens, output_tokens) {
                (Some(input), Some(output)) => Some(input + output),
                (Some(input), None) => Some(input),
                (None, Some(output)) => Some(output),
                (None, None) => None,
            }
        } else {
            total_tokens
        };

        Self {
            input_tokens,
            output_tokens,
            total_tokens: calculated_total,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        }
    }

    /// Attach cache-token counts to a `Usage`. `cache_read` / `cache_creation`
    /// are ADDITIVE to `input_tokens` (they must not already be folded into it),
    /// which keeps [`Usage::billed_total`] a plain sum of disjoint buckets.
    pub fn with_cache(mut self, cache_read: Option<i32>, cache_creation: Option<i32>) -> Self {
        self.cache_read_input_tokens = cache_read;
        self.cache_creation_input_tokens = cache_creation;
        self
    }

    /// The number of tokens this turn is billed for: the sum of every disjoint
    /// bucket — fresh input + output + cache-read + cache-creation. Returns
    /// `None` only when no bucket has a value (so a genuinely empty usage is not
    /// reported as `0`). Because the four buckets never overlap (see the field
    /// invariants), this is the number that reconciles with a vendor's billing
    /// dashboard — unlike `total_tokens`, which is context-window occupancy.
    pub fn billed_total(&self) -> Option<i64> {
        let has_any = self.input_tokens.is_some()
            || self.output_tokens.is_some()
            || self.cache_read_input_tokens.is_some()
            || self.cache_creation_input_tokens.is_some();
        if !has_any {
            return None;
        }
        Some(
            i64::from(self.input_tokens.unwrap_or(0))
                + i64::from(self.output_tokens.unwrap_or(0))
                + i64::from(self.cache_read_input_tokens.unwrap_or(0))
                + i64::from(self.cache_creation_input_tokens.unwrap_or(0)),
        )
    }
}

use async_trait::async_trait;

/// Trait for LeadWorkerProvider-specific functionality
pub trait LeadWorkerProviderTrait {
    /// Get information about the lead and worker models for logging
    fn get_model_info(&self) -> (String, String);

    /// Get the currently active model name
    fn get_active_model(&self) -> String;

    /// Get (lead_turns, failure_threshold, fallback_turns)
    fn get_settings(&self) -> (usize, usize, usize);

    /// Stable identity of this composite configuration. Routing snapshots keep
    /// this value while a newly selected lead/worker configuration gets a new
    /// one, allowing session persistence to reject stale turn completions.
    fn get_config_generation(&self) -> &str;
}

/// Base trait for AI providers (OpenAI, Anthropic, etc)
#[async_trait]
pub trait Provider: Send + Sync {
    /// Get the metadata for this provider type
    fn metadata() -> ProviderMetadata
    where
        Self: Sized;

    /// Get the name of this provider instance
    fn get_name(&self) -> &str;

    /// A secret-free recipe for reconstructing this exact resolved provider.
    /// Built-ins with mutable routes or commands override this; registry-backed
    /// providers retain their name and complete model configuration.
    fn restore_binding(&self) -> crate::providers::provider_binding::ProviderRestoreBinding {
        crate::providers::provider_binding::ProviderRestoreBinding::registry(
            self.get_name().to_string(),
            self.get_model_config(),
        )
    }

    /// The least-private component of what this **instance** actually resolved.
    ///
    /// An instance method, never a lookup on `get_name()`: `get_name()` on a
    /// composite returns the lead's name (see `LeadWorkerProvider`), and
    /// `providers::create` can hand back something other than what was asked
    /// for (the factory intercepts `BIOROUTER_LEAD_MODEL` *before* the registry
    /// lookup, so `create("ollama", ..)` can return a composite whose lead is
    /// `anthropic`).
    ///
    /// DEFAULT = Public. Fail-safe: a provider module that forgets this gets
    /// less reach, never more — and a custom declarative provider that shadows
    /// a built-in name (see `crates/biorouter/src/config/declarative_providers.rs`,
    /// which registers by `config.name` after the built-ins, so a JSON file named
    /// `versa_azure` overwrites the real entry) loses a badge rather than forging
    /// one.
    fn tier(&self) -> ProviderTier {
        ProviderTier::Public
    }

    /// Whose compliance regime covers what this **instance** actually resolved —
    /// DR-26's third axis.
    ///
    /// [`Self::tier`] asks *how sensitive is this?*; affiliation asks *under
    /// whose agreements?*, and the two do not compose. A HIPAA-compliant LLM
    /// approved at one institution has no blanket permission over another
    /// institution's PHI, so two endpoints can be equally Private and still be
    /// incompatible. `None` is a public model, for which affiliation never
    /// applies: the tier gates already keep it away from private data.
    ///
    /// An **instance** method for exactly the reasons `tier` is one, recorded
    /// above: `get_name()` on a composite returns the lead's name, and
    /// `providers::create` may hand back something other than what was asked
    /// for. It is also why both Versa providers derive `ucsf` from the same
    /// gateway-host check that decides their tier rather than from their name —
    /// a name-keyed table would keep claiming the institution for a module
    /// repointed elsewhere, which `tier()` had already demoted to Public.
    ///
    /// DEFAULT = `None`, fail-safe in the same direction as `tier`'s Public: a
    /// provider module that forgets this gets less reach, never more.
    /// [`ModelAffiliation::Local`] in particular is the *most* permissive value
    /// in the model — it reaches every private extension, because no transfer
    /// occurs at all — so it must never be reached by omission.
    fn affiliation(&self) -> Option<ModelAffiliation> {
        None
    }

    /// Whether this **instance** drives a child agent that must call back in over
    /// MCP to use Biorouter's tools.
    ///
    /// True for exactly `claude_code` and `codex`: they run their own loop in a
    /// child process, so a tool list in the request reaches nothing and the only
    /// channel that returns a tool *result* into their live turn is
    /// `providers::coding_agent::bridge`. Every other provider receives its tools
    /// in the request, and issuing a grant for one would leave a live capability
    /// on every turn in the process for nobody to use.
    ///
    /// An **instance** method for the same reason [`Self::tier`] and
    /// [`Self::affiliation`] are, and their doc comments carry the full argument:
    /// `get_name()` on a composite answers for the **lead alone**. Asking the name
    /// meant a lead/worker pair whose worker is a coding agent got no bridge —
    /// and the worker runs most of the turns, so the tool-less child was the
    /// common case rather than the corner one.
    ///
    /// DEFAULT = the name lookup, which is right for every non-composite provider
    /// because a provider instance built from a registry id *is* that id. The
    /// composite is the one shape where instance and name disagree, and it
    /// overrides. Note the direction of the two defaults differs from `tier`'s and
    /// `affiliation`'s deliberately: theirs are fail-safe *restrictions*, while a
    /// wrong answer here costs tools rather than granting reach — the grant is
    /// still tier-filtered, still inspected, and still one turn long.
    fn uses_tool_bridge(&self) -> bool {
        crate::providers::coding_agent::bridge::provider_uses_bridge(self.get_name())
    }

    /// Whether the provider selected for this completion must have its ordinary
    /// request-tool surface replaced by the coding-agent bridge roster.
    /// Composite providers may still return `true` from [`Self::uses_tool_bridge`]
    /// so a lease exists for another component while returning `false` here for
    /// the currently active ordinary component.
    fn uses_tool_bridge_for_tool_surface(&self) -> bool {
        self.uses_tool_bridge()
    }

    // Internal implementation of complete, used by complete_fast and complete
    // Providers should override this to implement their actual completion logic
    async fn complete_with_model(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError>;

    // Default implementation: use the provider's configured model
    async fn complete(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let model_config = self.get_model_config();
        self.complete_with_model(&model_config, system, messages, tools)
            .await
    }

    // Check if a fast model is configured, otherwise fall back to regular model
    async fn complete_fast(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let model_config = self.get_model_config();
        let fast_config = model_config.use_fast_model();

        match self
            .complete_with_model(&fast_config, system, messages, tools)
            .await
        {
            Ok(result) => Ok(result),
            Err(e) => {
                if fast_config.model_name != model_config.model_name {
                    tracing::warn!(
                        "Fast model {} failed with error: {}. Falling back to regular model {}",
                        fast_config.model_name,
                        e,
                        model_config.model_name
                    );
                    self.complete_with_model(&model_config, system, messages, tools)
                        .await
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Get the model config from the provider
    fn get_model_config(&self) -> ModelConfig;

    fn retry_config(&self) -> RetryConfig {
        RetryConfig::default()
    }

    async fn fetch_supported_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        Ok(None)
    }

    /// Fetch models filtered by canonical registry and usability
    async fn fetch_recommended_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        let all_models = match self.fetch_supported_models().await? {
            Some(models) => models,
            None => return Ok(None),
        };

        let registry = CanonicalModelRegistry::bundled().map_err(|e| {
            ProviderError::ExecutionError(format!("Failed to load canonical registry: {}", e))
        })?;

        let provider_name = self.get_name();

        let recommended_models: Vec<String> = all_models
            .iter()
            .filter(|model| {
                map_to_canonical_model(provider_name, model, registry)
                    .and_then(|canonical_id| registry.get(&canonical_id))
                    .map(|m| m.input_modalities.contains(&"text".to_string()))
                    .unwrap_or(false)
            })
            .cloned()
            .collect();

        if recommended_models.is_empty() {
            Ok(Some(all_models))
        } else {
            Ok(Some(recommended_models))
        }
    }

    async fn map_to_canonical_model(
        &self,
        provider_model: &str,
    ) -> Result<Option<String>, ProviderError> {
        let registry = CanonicalModelRegistry::bundled().map_err(|e| {
            ProviderError::ExecutionError(format!("Failed to load canonical registry: {}", e))
        })?;

        Ok(map_to_canonical_model(
            self.get_name(),
            provider_model,
            registry,
        ))
    }

    fn supports_embeddings(&self) -> bool {
        false
    }

    async fn supports_cache_control(&self) -> bool {
        false
    }

    /// Create embeddings if supported. Default implementation returns an error.
    async fn create_embeddings(&self, _texts: Vec<String>) -> Result<Vec<Vec<f32>>, ProviderError> {
        Err(ProviderError::ExecutionError(
            "This provider does not support embeddings".to_string(),
        ))
    }

    /// Check if this provider is a LeadWorkerProvider
    /// This is used for logging model information at startup
    fn as_lead_worker(&self) -> Option<&dyn LeadWorkerProviderTrait> {
        None
    }

    async fn stream(
        &self,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        Err(ProviderError::NotImplemented(
            "streaming not implemented".to_string(),
        ))
    }

    /// Stream a turn while accepting user instructions that must reach the
    /// provider's already-running turn.
    ///
    /// The default deliberately closes the steering receiver and falls back to
    /// ordinary streaming. Callers must check [`Self::supports_live_steering`]
    /// before exposing the sender: accepting a request into a local queue is not
    /// the same as delivering it to a provider.
    async fn stream_with_steering(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
        steering: ProviderSteerReceiver,
    ) -> Result<MessageStream, ProviderError> {
        drop(steering);
        self.stream(system, messages, tools).await
    }

    fn supports_streaming(&self) -> bool {
        false
    }

    /// Whether [`Self::stream_with_steering`] delivers instructions into the
    /// provider's current turn and acknowledges the provider transport.
    fn supports_live_steering(&self) -> bool {
        false
    }

    /// Whether a queued mid-turn instruction can be delivered by abandoning the
    /// current response stream and immediately reissuing the turn with that
    /// instruction in the conversation.
    ///
    /// This is deliberately distinct from [`Self::supports_live_steering`]. A
    /// live-steering provider keeps one request running and transports the steer
    /// inside it; a restart-steering provider has no such transport and relies on
    /// dropping the in-flight stream to cancel the request. The default is false
    /// because not every streaming transport documents cancellation-on-drop as a
    /// safe way to restart a turn after partial output has been emitted.
    fn supports_restart_steering(&self) -> bool {
        false
    }

    /// Whether the `tools` argument of [`Provider::complete`] actually reaches
    /// the model — i.e. whether this provider can drive a **tool-calling loop
    /// that Biorouter itself runs**, such as the knowledge sub-agent behind the
    /// ingest / query / lint macros.
    ///
    /// True for ordinary API providers and for coding-agent providers. The
    /// latter do not forward the `tools` argument themselves, but
    /// `ProviderCompleter::complete_with_dispatch` gives their child a scoped
    /// MCP bridge to the loop's dispatcher. A provider that has neither request
    /// tool calls nor an equivalent bridge overrides this to `false`.
    ///
    /// ⚠ It is a capability question, never a name check. Ask this, and report
    /// the mismatch; do not silently substitute a different provider, which
    /// would move the user's inference onto a different bill.
    ///
    /// A provider that gains the ability flips this to `true` and every caller
    /// follows — there is no second list to update.
    fn supports_tool_calls(&self) -> bool {
        true
    }

    /// Get the currently active model name
    /// For regular providers, this returns the configured model
    /// For LeadWorkerProvider, this returns the currently active model (lead or worker)
    fn get_active_model_name(&self) -> String {
        if let Some(lead_worker) = self.as_lead_worker() {
            lead_worker.get_active_model()
        } else {
            self.get_model_config().model_name
        }
    }

    /// Returns the first 3 user messages as strings for session naming
    fn get_initial_user_messages(&self, messages: &Conversation) -> Vec<String> {
        messages
            .iter()
            .filter(|m| m.role == rmcp::model::Role::User)
            .take(MSG_COUNT_FOR_SESSION_NAME_GENERATION)
            .map(|m| m.as_concat_text())
            .collect()
    }

    /// Generate a session name/description based on the conversation history
    /// Creates a prompt asking for a concise description in 4 words or less.
    async fn generate_session_name(
        &self,
        messages: &Conversation,
    ) -> Result<String, ProviderError> {
        let context = self.get_initial_user_messages(messages);
        let prompt = self.create_session_name_prompt(&context);
        let message = Message::user().with_text(&prompt);
        let result = self
            .complete_fast(
                "Reply with only a description in four words or less",
                &[message],
                &[],
            )
            .await?;

        let description = result
            .0
            .as_concat_text()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        Ok(safe_truncate(&description, 100))
    }

    // Generate a prompt for a session name based on the conversation history
    fn create_session_name_prompt(&self, context: &[String]) -> String {
        // Create a prompt for a concise description
        let mut prompt = "Based on the conversation so far, provide a concise description of this session in 4 words or less. This will be used for finding the session later in a UI with limited space - reply *ONLY* with the description".to_string();

        if !context.is_empty() {
            prompt = format!(
                "Here are the first few user messages:\n{}\n\n{}",
                context.join("\n"),
                prompt
            );
        }
        prompt
    }

    /// Configure OAuth authentication for this provider
    ///
    /// This method is called when a provider has configuration keys marked with oauth_flow = true.
    /// Providers that support OAuth should override this method to implement their specific OAuth flow.
    ///
    /// # Returns
    /// * `Ok(())` if OAuth configuration succeeds and credentials are saved
    /// * `Err(ProviderError)` if OAuth fails or is not supported by this provider
    ///
    /// # Default Implementation
    /// The default implementation returns an error indicating OAuth is not supported.
    async fn configure_oauth(&self) -> Result<(), ProviderError> {
        Err(ProviderError::ExecutionError(
            "OAuth configuration not supported by this provider".to_string(),
        ))
    }
}

/// A tool call the provider has *started* emitting but has not finished.
///
/// # This is deliberately NOT a `MessageContent::ToolRequest`
///
/// A tool request that reaches the agent's dispatch path is executed. A
/// *partial* tool request would be executed with truncated arguments — for
/// `shell` or `text_editor` that destroys user data, and it would happen once
/// per streamed delta. Modelling pending state as a `Message` therefore cannot
/// be made safe by convention; the only durable guarantee is a structural one.
///
/// `PendingToolCall` travels on its own channel, parallel to `Message`, all the
/// way to the frontend. `categorize_tools`, `num_tool_requests`, dispatch,
/// session persistence and replay all walk `Message` contents exclusively, so a
/// value of this type is *incapable* of being executed, gated, persisted or
/// replayed. It exists only to let the UI draw a card the moment the tool's
/// name is known, seconds before its arguments finish generating.
///
/// Invariant 1 of the investigation (§6.5) is upheld by this type's existence,
/// not by any check. See `crates/biorouter/tests/streaming_pending_tool_calls.rs`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PendingToolCall {
    /// Provider-assigned tool-call id. Identical to the id on the authoritative
    /// `ToolRequest` that follows, so the frontend can upsert by id.
    pub id: String,
    /// The tool name. Known at `content_block_start` / the first tool-call
    /// chunk — i.e. before any argument bytes exist.
    pub name: String,
    /// Arguments accumulated *so far*. Almost never valid JSON. Emitted
    /// throttled (never per delta) and purely for display; nothing may parse
    /// this and act on it.
    pub partial_args: Option<String>,
}

/// One item of a provider stream: an optional partial/complete `Message`, an
/// optional usage snapshot, and an optional [`PendingToolCall`] notification.
///
/// The third slot is a *notification only*. Consumers must forward it and must
/// never fold it into a `Message`.
pub type ProviderStreamItem = (
    Option<Message>,
    Option<ProviderUsage>,
    Option<PendingToolCall>,
);

/// §6.2b kill switch: batch a single response's `tool_use` / `functionCall`
/// blocks into **one** assistant `Message` carrying N `ToolRequest`s, instead of
/// one `Message` per block. The agent's `select_all` then sees N tool futures at
/// once and dispatches them in parallel; one message per block made multi-tool
/// turns run serially on the native Anthropic/Google decoders (the OpenAI
/// decoder already batches).
///
/// On by default; only an explicit `0`/`false`/`no`/`off` in
/// `BIOROUTER_TOOL_CALL_BATCHING` restores the pre-6.2b one-message-per-tool
/// behaviour (serial execution) as a full rollback — mirroring
/// `BIOROUTER_TOOL_WRITE_ORDERING`. This flag changes only *execution timing*:
/// the batch is re-split into one assistant message per request (fresh uuid)
/// before it is persisted or replayed (`agent.rs`), so session history is
/// identical either way.
pub fn tool_call_batching_enabled() -> bool {
    match std::env::var("BIOROUTER_TOOL_CALL_BATCHING") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

/// A message stream yields partial text content but complete tool calls, all within the Message object
/// So a message with text will contain potentially just a word of a longer response, but tool calls
/// messages will only be yielded once concatenated.
pub type MessageStream =
    Pin<Box<dyn Stream<Item = Result<ProviderStreamItem, ProviderError>> + Send>>;

/// A user instruction destined for a provider turn that is already running.
///
/// The acknowledgement is intentionally owned by the request. It resolves only
/// after the provider-specific transport accepts the instruction, so callers
/// can distinguish a real mid-turn steer from text that was merely queued in
/// Biorouter.
pub struct ProviderSteerRequest {
    text: String,
    acknowledgement: oneshot::Sender<Result<(), ProviderError>>,
}

impl ProviderSteerRequest {
    pub fn new(text: impl Into<String>) -> (Self, oneshot::Receiver<Result<(), ProviderError>>) {
        let (acknowledgement, acknowledged) = oneshot::channel();
        (
            Self {
                text: text.into(),
                acknowledgement,
            },
            acknowledged,
        )
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn acknowledge(self) {
        let _ = self.acknowledgement.send(Ok(()));
    }

    pub fn reject(self, error: ProviderError) {
        let _ = self.acknowledgement.send(Err(error));
    }
}

pub type ProviderSteerSender = mpsc::UnboundedSender<ProviderSteerRequest>;
pub type ProviderSteerReceiver = mpsc::UnboundedReceiver<ProviderSteerRequest>;

pub fn provider_steer_channel() -> (ProviderSteerSender, ProviderSteerReceiver) {
    mpsc::unbounded_channel()
}

#[cfg(test)]
mod provider_steering_tests {
    use super::*;

    #[tokio::test]
    async fn acknowledgement_resolves_only_when_the_provider_accepts_the_request() {
        let (sender, mut receiver) = provider_steer_channel();
        let (request, mut acknowledged) = ProviderSteerRequest::new("change course");
        assert!(sender.send(request).is_ok(), "receiver is live");

        let delivered = receiver.recv().await.expect("request was queued");
        assert_eq!(delivered.text(), "change course");
        assert!(
            acknowledged.try_recv().is_err(),
            "queueing alone must not claim provider acceptance"
        );

        delivered.acknowledge();
        acknowledged
            .await
            .expect("ack sender remained live")
            .expect("provider accepted the steer");
    }
}

pub fn stream_from_single_message(message: Message, usage: ProviderUsage) -> MessageStream {
    let stream = futures::stream::once(async move { Ok((Some(message), Some(usage), None)) });
    Box::pin(stream)
}

#[cfg(test)]
mod type_level_institution_tests {
    //! [`ProviderMetadata::institutions`] — the *type-level* institution claim.
    //!
    //! It exists because `GET /config/providers` resolves the instance-level
    //! [`ProviderDetails::affiliation`] only for a **configured** provider, so on
    //! a machine where nothing is set up yet every row's affiliation is `null`.
    //! A catalog that groups private gateways by institution would then have
    //! nothing to group by on exactly the screen — first-run onboarding — where
    //! naming the institution matters most.

    use super::*;

    #[test]
    fn a_provider_declares_no_institution_by_default() {
        // The direction that matters: forgetting to declare one costs a group
        // heading, never invents an institutional claim for a provider that
        // ships pointed nowhere in particular.
        assert!(ProviderMetadata::empty().institutions.is_empty());
    }

    #[test]
    fn a_declared_institution_carries_the_registry_display_name() {
        // Both halves travel, and the name comes from the SAME registry every
        // warning and badge reads (`institution_display_name`) rather than from
        // a literal in the provider module — two spellings of "UCSF" is exactly
        // the drift `AffiliationInstitution` exists to prevent.
        let meta = ProviderMetadata::empty().with_institution("ucsf");
        assert_eq!(
            meta.institutions,
            vec![AffiliationInstitution {
                id: "ucsf".to_string(),
                display_name: Some("UCSF".to_string()),
            }]
        );
    }

    #[test]
    fn an_unpublished_institution_keeps_its_bare_id_rather_than_disappearing() {
        // Mirrors `privacy::affiliation::label`: a real constraint must never
        // vanish from a surface because nobody wrote it a pretty name. A
        // renderer that dropped the row would hide the group entirely.
        let meta = ProviderMetadata::empty().with_institution("not-in-the-registry");
        assert_eq!(meta.institutions.len(), 1);
        assert_eq!(meta.institutions[0].id, "not-in-the-registry");
        assert_eq!(meta.institutions[0].display_name, None);
    }

    #[test]
    fn the_id_is_normalised_by_the_same_normaliser_the_gates_use() {
        // `InstitutionId::new` interns through `name_to_key`. Declaring "UCSF"
        // and declaring "ucsf" must produce the same id, or a catalog would show
        // two groups for one institution while the gates saw one.
        assert_eq!(
            ProviderMetadata::empty().with_institution("UCSF").institutions,
            ProviderMetadata::empty().with_institution("ucsf").institutions
        );
    }

    /// ⚠ The field is `institutions`, **not** `affiliation`, and that is load
    /// bearing rather than cosmetic: `config_management`'s
    /// `the_affiliation_travels_beside_the_metadata_not_inside_it` asserts that
    /// `metadata` carries no `affiliation` key, because a renderer reading
    /// `row.metadata.affiliation` would silently find an unresolved type-level
    /// claim where it asked for an instance-resolved one.
    #[test]
    fn the_type_level_claim_does_not_serialise_under_the_instance_level_key() {
        let json = serde_json::to_value(ProviderMetadata::empty().with_institution("ucsf")).unwrap();
        assert!(json.get("affiliation").is_none());
        assert_eq!(json["institutions"][0]["id"], "ucsf");
        assert_eq!(json["institutions"][0]["display_name"], "UCSF");
    }

    /// Metadata from a daemon that predates the field still reads — the
    /// `#[serde(default)]` that keeps the addition non-breaking for every
    /// fixture round-tripping this type.
    #[test]
    fn metadata_without_the_field_still_reads() {
        let parsed: ProviderMetadata = serde_json::from_value(serde_json::json!({
            "name": "openai",
            "display_name": "OpenAI",
            "description": "",
            "default_model": "",
            "known_models": [],
            "model_doc_link": "",
            "config_keys": [],
        }))
        .expect("metadata predating the field is still metadata");
        assert!(parsed.institutions.is_empty());
    }
}

#[cfg(test)]
mod affiliation_view_tests {
    //! DR-26's third axis as `GET /config/providers` renders it (issue #56).
    //!
    //! One named test per row of [`ProviderAffiliation::resolved`]'s table, plus
    //! the three states the task brief calls out as the ones a naive mapping
    //! gets wrong. The rows are the whole specification; anything else here
    //! exists to catch a mapping that satisfies the table and is still wrong.

    use super::*;
    use crate::privacy::affiliation::{InstitutionId, ModelAffiliation};
    use async_trait::async_trait;

    fn ucsf() -> ModelAffiliation {
        ModelAffiliation::institution(InstitutionId::new("ucsf"))
    }

    /// Row 1: a public model has **no** affiliation — not an empty one, and not
    /// `Unstated`.
    ///
    /// Breaking the tier guard in `resolved` (dropping the early return, so a
    /// public model falls through to the `affiliation()` match) fails here and
    /// nowhere else: every other row is about a private model.
    #[test]
    fn a_public_provider_has_no_affiliation_at_all() {
        assert_eq!(
            ProviderAffiliation::resolved(ProviderTier::Public, None),
            None
        );
        // ...and still nothing even if the instance somehow states one. The
        // gates ask the tier first (`gate_cross_affiliation`'s first line), so
        // the badge must too, or Public and `Unstated` collapse into one chip.
        assert_eq!(
            ProviderAffiliation::resolved(ProviderTier::Public, Some(ucsf())),
            None
        );
        assert_eq!(
            ProviderAffiliation::resolved(ProviderTier::Public, Some(ModelAffiliation::Local)),
            None
        );
    }

    /// Row 2: `Local`. Its own kind, with no institutions — never folded onto
    /// `Unstated` because both happen to name nobody.
    #[test]
    fn a_local_provider_is_local_and_not_unstated() {
        let view =
            ProviderAffiliation::resolved(ProviderTier::Private, Some(ModelAffiliation::Local))
                .expect("a private local model has an affiliation to show");
        assert_eq!(view.kind, ProviderAffiliationKind::Local);
        assert!(view.institutions.is_empty());
        assert_ne!(view.kind, ProviderAffiliationKind::Unstated);
    }

    /// Row 3: institutions, carried with the registry's display name beside the
    /// raw id so a renderer never has to choose between them.
    #[test]
    fn an_institution_provider_carries_the_id_and_the_published_name() {
        let view = ProviderAffiliation::resolved(ProviderTier::Private, Some(ucsf()))
            .expect("a private institutional model has an affiliation to show");
        assert_eq!(view.kind, ProviderAffiliationKind::Institutions);
        assert_eq!(view.institutions.len(), 1);
        assert_eq!(view.institutions[0].id, "ucsf");
    }

    /// An institution the registry does not publish keeps its raw id rather than
    /// vanishing (Task 47: an unpublished institution is a *mismatch* surfaced
    /// raw, never silently dropped).
    #[test]
    fn an_unpublished_institution_still_renders_as_its_raw_id() {
        let obscure =
            ModelAffiliation::institution(InstitutionId::new("no-such-institution-in-registry"));
        let view = ProviderAffiliation::resolved(ProviderTier::Private, Some(obscure))
            .expect("an institutional model has an affiliation to show");
        assert_eq!(view.institutions.len(), 1);
        assert_eq!(view.institutions[0].id, "no-such-institution-in-registry");
        assert_eq!(view.institutions[0].display_name, None);
    }

    /// A model covered by two institutions renders **both**, ascending. Picking
    /// a representative is what `InstitutionSet::sole` refuses to do for the
    /// gates, and a badge that named one of the two would tell the user the
    /// chat is covered by an institution it only half is.
    #[test]
    fn a_spanning_provider_renders_every_institution_in_a_stable_order() {
        let spans = ModelAffiliation::institutions([
            InstitutionId::new("ucsf"),
            InstitutionId::new("stanford"),
        ])
        .expect("two institutions is a representable affiliation");
        let view = ProviderAffiliation::resolved(ProviderTier::Private, Some(spans))
            .expect("a private institutional model has an affiliation to show");
        let ids: Vec<&str> = view.institutions.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["stanford", "ucsf"]);
    }

    /// Row 4, and the state the brief calls out: a **private** model that states
    /// nothing is `Unstated`, which is a fact, not an absence. Folding it to
    /// `None` would show the least-reaching private model (it clears only
    /// unconstrained extensions) as the one with no constraint at all.
    #[test]
    fn a_private_provider_that_states_nothing_is_unstated_not_absent() {
        let view = ProviderAffiliation::resolved(ProviderTier::Private, None)
            .expect("a private model with no stated institution still has something to say");
        assert_eq!(view.kind, ProviderAffiliationKind::Unstated);
        assert!(view.institutions.is_empty());
    }

    /// A provider whose module states neither axis. The trait defaults are
    /// Public + `None`, so it renders nothing — fail-safe in the same direction
    /// `tier()`'s and `affiliation()`'s defaults are.
    struct SaysNothing;

    #[async_trait]
    impl Provider for SaysNothing {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::empty()
        }
        fn get_name(&self) -> &str {
            "says-nothing"
        }
        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail("test-model")
        }
        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            unreachable!("this fixture exists only to be asked its two privacy axes")
        }
    }

    /// A provider that resolved an institution, and one whose endpoint moved.
    /// The pair is what pins `of` to the INSTANCE: both have the same type and
    /// the same `get_name()`, so any name-keyed lookup answers identically for
    /// the two and this test fails.
    struct Resolved {
        tier: ProviderTier,
        affiliation: Option<ModelAffiliation>,
    }

    #[async_trait]
    impl Provider for Resolved {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::empty()
        }
        fn get_name(&self) -> &str {
            // Deliberately identical for both instances below.
            "versa_azure"
        }
        fn tier(&self) -> ProviderTier {
            self.tier
        }
        fn affiliation(&self) -> Option<ModelAffiliation> {
            self.affiliation
        }
        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail("test-model")
        }
        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            unreachable!("this fixture exists only to be asked its two privacy axes")
        }
    }

    #[test]
    fn a_provider_that_states_neither_axis_renders_nothing() {
        assert_eq!(ProviderAffiliation::of(&SaysNothing), None);
    }

    #[test]
    fn restart_steering_is_opt_in() {
        assert!(!SaysNothing.supports_restart_steering());
    }

    /// ⚠ **The point of the whole task.** Two instances of one provider *type*
    /// with one `get_name()`: the shipped one on the UCSF gateway, and one whose
    /// endpoint was repointed. The repointed instance must lose Private and
    /// `ucsf` **together** — a name-keyed table would hand both instances
    /// `ucsf`, and this is the assertion that catches it.
    #[test]
    fn a_repointed_instance_loses_private_and_its_institution_together() {
        let shipped = Resolved {
            tier: ProviderTier::Private,
            affiliation: Some(ucsf()),
        };
        let repointed = Resolved {
            tier: ProviderTier::Public,
            affiliation: None,
        };
        assert_eq!(shipped.get_name(), repointed.get_name());

        let shipped_view =
            ProviderAffiliation::of(&shipped).expect("the shipped instance is UCSF's");
        assert_eq!(shipped_view.kind, ProviderAffiliationKind::Institutions);
        assert_eq!(shipped_view.institutions[0].id, "ucsf");

        assert_eq!(
            ProviderAffiliation::of(&repointed),
            None,
            "a repointed Versa is Public, so it has no institution to claim"
        );
    }

    /// The wire shape the renderer parses, pinned so a serde rename does not
    /// silently turn every badge into the unrecognised-kind fallback.
    #[test]
    fn the_wire_words_are_the_ones_the_renderer_switches_on() {
        let kinds = [
            (ProviderAffiliationKind::Local, "\"local\""),
            (ProviderAffiliationKind::Institutions, "\"institutions\""),
            (ProviderAffiliationKind::Unstated, "\"unstated\""),
        ];
        for (kind, expected) in kinds {
            assert_eq!(serde_json::to_string(&kind).unwrap(), expected);
        }
        let view = ProviderAffiliation::resolved(ProviderTier::Private, Some(ucsf())).unwrap();
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["kind"], "institutions");
        assert_eq!(json["institutions"][0]["id"], "ucsf");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use serde_json::json;
    #[test]
    fn test_usage_creation() {
        let usage = Usage::new(Some(10), Some(20), Some(30));
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.output_tokens, Some(20));
        assert_eq!(usage.total_tokens, Some(30));
    }

    #[test]
    fn billed_total_sums_all_four_disjoint_buckets() {
        // 100 fresh input + 50 output + 900 cache-read + 200 cache-creation
        // = 1250 billed tokens. total_tokens (context occupancy) is separate.
        let usage = Usage::new(Some(100), Some(50), Some(1250)).with_cache(Some(900), Some(200));
        assert_eq!(usage.billed_total(), Some(1250));
    }

    #[test]
    fn billed_total_without_cache_is_input_plus_output() {
        let usage = Usage::new(Some(100), Some(50), Some(150));
        assert_eq!(usage.billed_total(), Some(150));
    }

    #[test]
    fn billed_total_treats_missing_buckets_as_zero_but_all_missing_as_none() {
        // Only cache present: still billed (not None), and input/output count 0.
        let only_cache = Usage::default().with_cache(Some(500), None);
        assert_eq!(only_cache.billed_total(), Some(500));
        // Nothing at all -> None (an empty usage must not read as $0/0 tokens).
        assert_eq!(Usage::default().billed_total(), None);
    }

    #[test]
    fn adding_usages_sums_cache_buckets_too() {
        let a = Usage::new(Some(10), Some(1), Some(31)).with_cache(Some(20), None);
        let b = Usage::new(Some(5), Some(2), Some(107)).with_cache(Some(100), Some(3));
        let sum = a + b;
        assert_eq!(sum.input_tokens, Some(15));
        assert_eq!(sum.output_tokens, Some(3));
        assert_eq!(sum.total_tokens, Some(138));
        assert_eq!(sum.cache_read_input_tokens, Some(120));
        assert_eq!(sum.cache_creation_input_tokens, Some(3));
        assert_eq!(sum.billed_total(), Some(15 + 3 + 120 + 3));
    }

    #[test]
    fn old_usage_json_without_cache_fields_deserializes_to_none() {
        // A token row persisted before Phase 4 has no cache keys.
        let legacy = r#"{"input_tokens":10,"output_tokens":20,"total_tokens":30}"#;
        let usage: Usage = serde_json::from_str(legacy).unwrap();
        assert_eq!(usage.cache_read_input_tokens, None);
        assert_eq!(usage.cache_creation_input_tokens, None);
        assert_eq!(usage.billed_total(), Some(30));
    }

    #[test]
    fn test_usage_serialization() -> Result<()> {
        let usage = Usage::new(Some(10), Some(20), Some(30));
        let serialized = serde_json::to_string(&usage)?;
        let deserialized: Usage = serde_json::from_str(&serialized)?;

        assert_eq!(usage.input_tokens, deserialized.input_tokens);
        assert_eq!(usage.output_tokens, deserialized.output_tokens);
        assert_eq!(usage.total_tokens, deserialized.total_tokens);

        // Test JSON structure
        let json_value: serde_json::Value = serde_json::from_str(&serialized)?;
        assert_eq!(json_value["input_tokens"], json!(10));
        assert_eq!(json_value["output_tokens"], json!(20));
        assert_eq!(json_value["total_tokens"], json!(30));

        Ok(())
    }

    #[test]
    fn test_set_and_get_current_model() {
        // Set the model
        set_current_model("gpt-4o");

        // Get the model and verify
        let model = get_current_model();
        assert_eq!(model, Some("gpt-4o".to_string()));

        // Change the model
        set_current_model("claude-sonnet-4-20250514");

        // Get the updated model and verify
        let model = get_current_model();
        assert_eq!(model, Some("claude-sonnet-4-20250514".to_string()));
    }

    #[test]
    fn test_provider_metadata_context_limits() {
        // Test that ProviderMetadata::new correctly sets context limits
        let test_models = vec!["gpt-4o", "claude-sonnet-4-20250514", "unknown-model"];
        let metadata = ProviderMetadata::new(
            "test",
            "Test Provider",
            "Test Description",
            "gpt-4o",
            test_models,
            "https://example.com",
            vec![],
        );

        let model_info: HashMap<String, usize> = metadata
            .known_models
            .into_iter()
            .map(|m| (m.name, m.context_limit))
            .collect();

        // gpt-4o should have 128k limit
        assert_eq!(*model_info.get("gpt-4o").unwrap(), 128_000);

        // claude-sonnet-4-20250514 should have 200k limit
        assert_eq!(
            *model_info.get("claude-sonnet-4-20250514").unwrap(),
            200_000
        );

        // unknown model should have default limit (128k)
        assert_eq!(*model_info.get("unknown-model").unwrap(), 128_000);
    }

    #[test]
    fn test_model_info_creation() {
        // Test direct ModelInfo creation
        let info = ModelInfo {
            name: "test-model".to_string(),
            context_limit: 1000,
            input_token_cost: None,
            output_token_cost: None,
            currency: None,
            supports_cache_control: None,
            supports_vision: None,
            supported_input_mime_types: None,
        };
        assert_eq!(info.context_limit, 1000);

        // Test equality
        let info2 = ModelInfo {
            name: "test-model".to_string(),
            context_limit: 1000,
            input_token_cost: None,
            output_token_cost: None,
            currency: None,
            supports_cache_control: None,
            supports_vision: None,
            supported_input_mime_types: None,
        };
        assert_eq!(info, info2);

        // Test inequality
        let info3 = ModelInfo {
            name: "test-model".to_string(),
            context_limit: 2000,
            input_token_cost: None,
            output_token_cost: None,
            currency: None,
            supports_cache_control: None,
            supports_vision: None,
            supported_input_mime_types: None,
        };
        assert_ne!(info, info3);
    }

    #[test]
    fn test_model_info_with_cost() {
        let info = ModelInfo::with_cost("gpt-4o", 128000, 0.0000025, 0.00001);
        assert_eq!(info.name, "gpt-4o");
        assert_eq!(info.context_limit, 128000);
        assert_eq!(info.input_token_cost, Some(0.0000025));
        assert_eq!(info.output_token_cost, Some(0.00001));
        assert_eq!(info.currency, Some("$".to_string()));
    }

    #[test]
    fn test_with_vision_sets_flag() {
        let info = ModelInfo::new("claude-3-5-sonnet", 200_000).with_vision();
        assert_eq!(info.supports_vision, Some(true));
        assert_eq!(
            info.supported_input_mime_types,
            Some(vec![
                "image/png".to_string(),
                "image/jpeg".to_string(),
                "image/jpg".to_string(),
                "image/gif".to_string(),
                "image/webp".to_string()
            ])
        );
    }

    #[test]
    fn test_default_vision_is_none() {
        let info = ModelInfo::new("text-only-model", 8_000);
        assert_eq!(info.supports_vision, None);
        assert_eq!(info.supported_input_mime_types, None);
    }

    #[test]
    fn known_vision_models_have_supports_vision_true() {
        use crate::providers::anthropic::AnthropicProvider;
        use crate::providers::azure::AzureProvider;
        use crate::providers::bedrock::BedrockProvider;
        use crate::providers::databricks::DatabricksProvider;
        use crate::providers::gcpvertexai::GcpVertexAIProvider;
        use crate::providers::githubcopilot::GithubCopilotProvider;
        use crate::providers::google::GoogleProvider;
        use crate::providers::openai::OpenAiProvider;
        use crate::providers::openrouter::OpenRouterProvider;
        use crate::providers::tetrate::TetrateProvider;
        use crate::providers::versa_azure::VersaAzureProvider;
        use crate::providers::versa_bedrock::VersaBedrockProvider;
        use crate::providers::xai::XaiProvider;

        // Note: Xiaomi MiMo is intentionally NOT covered here — its catalog has no
        // vision-capable model. Only "omni" MiMo models accept image input; the
        // text models (mimo-v2.5 / -pro, mimo-v2-pro) return 404 for images
        // (live-verified — see xiaomi_mimo.rs `model_supports_vision`), so none of
        // the known_models declare supports_vision: true.
        let cases: Vec<(ProviderMetadata, &str, &str)> = vec![
            (
                AnthropicProvider::metadata(),
                "claude-sonnet-4-6",
                "Anthropic Claude Sonnet 4.6",
            ),
            (OpenAiProvider::metadata(), "gpt-5.5", "OpenAI GPT-5.5"),
            (
                AzureProvider::metadata(),
                "gpt-5.5-2026-04-24",
                "Azure OpenAI GPT-5.5 deployment",
            ),
            (
                VersaAzureProvider::metadata(),
                "gpt-5.5-2026-04-24",
                "Versa Azure GPT-5.5 deployment",
            ),
            (
                GoogleProvider::metadata(),
                "gemini-2.5-pro",
                "Google Gemini 2.5 Pro",
            ),
            (
                BedrockProvider::metadata(),
                "us.anthropic.claude-opus-4-6-v1",
                "Amazon Bedrock Claude Opus 4.6",
            ),
            (
                VersaBedrockProvider::metadata(),
                "us.anthropic.claude-opus-4-6-v1",
                "Versa Bedrock Claude Opus 4.6",
            ),
            (
                GcpVertexAIProvider::metadata(),
                "claude-sonnet-4-6",
                "GCP Vertex AI Claude Sonnet 4.6",
            ),
            (
                GcpVertexAIProvider::metadata(),
                "gemini-3.5-flash",
                "GCP Vertex AI Gemini 3.5 Flash",
            ),
            (
                DatabricksProvider::metadata(),
                "databricks-claude-sonnet-4-6",
                "Databricks Claude Sonnet 4.6",
            ),
            (
                DatabricksProvider::metadata(),
                "databricks-llama-4-maverick",
                "Databricks Llama 4 Maverick",
            ),
            (
                OpenRouterProvider::metadata(),
                "anthropic/claude-sonnet-4.6",
                "OpenRouter Claude Sonnet 4.6",
            ),
            (
                OpenRouterProvider::metadata(),
                "x-ai/grok-4.3",
                "OpenRouter Grok 4.3",
            ),
            (
                TetrateProvider::metadata(),
                "claude-sonnet-4-6",
                "Tetrate Claude Sonnet 4.6",
            ),
            (
                GithubCopilotProvider::metadata(),
                "claude-sonnet-4.6",
                "GitHub Copilot Claude Sonnet 4.6",
            ),
            (XaiProvider::metadata(), "grok-4.3", "xAI Grok 4.3"),
        ];

        for (metadata, model_name, label) in cases {
            let info = metadata
                .known_models
                .iter()
                .find(|m| m.name == model_name)
                .unwrap_or_else(|| panic!("model {model_name} not in known_models for {label}"));
            assert_eq!(
                info.supports_vision,
                Some(true),
                "{label} should declare supports_vision: true"
            );
            assert!(
                info.supported_input_mime_types
                    .as_ref()
                    .is_some_and(|mime_types| mime_types.iter().any(|mime| mime == "image/png")),
                "{label} should declare uploadable image MIME types"
            );
        }
    }

    #[test]
    fn text_only_provider_does_not_claim_vision() {
        use crate::providers::githubcopilot::GithubCopilotProvider;
        use crate::providers::ollama::OllamaProvider;
        let metadata = OllamaProvider::metadata();
        for info in &metadata.known_models {
            assert_ne!(
                info.supports_vision,
                Some(true),
                "Ollama default-listed model {} should not claim vision (user overrides in config)",
                info.name
            );
        }

        let copilot_codex = GithubCopilotProvider::metadata()
            .known_models
            .into_iter()
            .find(|m| m.name == "gpt-5.3-codex")
            .expect("GitHub Copilot Codex model should be present");
        assert_eq!(copilot_codex.supports_vision, None);
    }

    #[test]
    fn xai_vision_models_are_limited_to_png_and_jpeg() {
        use crate::providers::xai::XaiProvider;
        let grok = XaiProvider::metadata()
            .known_models
            .into_iter()
            .find(|m| m.name == "grok-4.3")
            .expect("xAI Grok 4.3 should be present");
        assert_eq!(grok.supports_vision, Some(true));
        assert_eq!(
            grok.supported_input_mime_types,
            Some(vec![
                "image/png".to_string(),
                "image/jpeg".to_string(),
                "image/jpg".to_string()
            ])
        );
    }
}
