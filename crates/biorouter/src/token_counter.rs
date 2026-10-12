use ahash::AHasher;
use dashmap::DashMap;
use rmcp::model::Tool;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tiktoken_rs::CoreBPE;
use tokio::sync::OnceCell;

use crate::conversation::message::Message;

static TOKENIZER: OnceCell<Result<Arc<CoreBPE>, String>> = OnceCell::const_new();

const MAX_TOKEN_CACHE_SIZE: usize = 10_000;

// token use for various bits of a tool calls:
const FUNC_INIT: usize = 7;
const PROP_INIT: usize = 3;
const PROP_KEY: usize = 3;
const ENUM_INIT: isize = -3;
const ENUM_ITEM: usize = 3;
const FUNC_END: usize = 12;

pub struct TokenCounter {
    tokenizer: Arc<CoreBPE>,
    token_cache: Arc<DashMap<u64, usize>>,
}

impl TokenCounter {
    pub async fn new() -> Result<Self, String> {
        let tokenizer = get_tokenizer().await?;
        Ok(Self {
            tokenizer,
            token_cache: Arc::new(DashMap::new()),
        })
    }

    pub fn count_tokens(&self, text: &str) -> usize {
        let mut hasher = AHasher::default();
        text.hash(&mut hasher);
        let hash = hasher.finish();

        if let Some(count) = self.token_cache.get(&hash) {
            return *count;
        }

        let tokens = self.tokenizer.encode_with_special_tokens(text);
        let count = tokens.len();

        if self.token_cache.len() >= MAX_TOKEN_CACHE_SIZE {
            if let Some(entry) = self.token_cache.iter().next() {
                let old_hash = *entry.key();
                self.token_cache.remove(&old_hash);
            }
        }

        self.token_cache.insert(hash, count);
        count
    }

    pub fn count_tokens_for_tools(&self, tools: &[Tool]) -> usize {
        let mut func_token_count = 0;
        if !tools.is_empty() {
            for tool in tools {
                func_token_count += FUNC_INIT;
                let name = &tool.name;
                let description = &tool
                    .description
                    .as_ref()
                    .map(|d| d.as_ref())
                    .unwrap_or_default()
                    .trim_end_matches('.');

                let line = format!("{}:{}", name, description);
                func_token_count += self.count_tokens(&line);

                if let Some(serde_json::Value::Object(properties)) =
                    tool.input_schema.get("properties")
                {
                    if !properties.is_empty() {
                        func_token_count += PROP_INIT;
                        for (key, value) in properties {
                            func_token_count += PROP_KEY;
                            let p_name = key;
                            let p_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            let p_desc = value
                                .get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .trim_end_matches('.');

                            let line = format!("{}:{}:{}", p_name, p_type, p_desc);
                            func_token_count += self.count_tokens(&line);

                            if let Some(enum_values) = value.get("enum").and_then(|v| v.as_array())
                            {
                                func_token_count =
                                    func_token_count.saturating_add_signed(ENUM_INIT);
                                for item in enum_values {
                                    if let Some(item_str) = item.as_str() {
                                        func_token_count += ENUM_ITEM;
                                        func_token_count += self.count_tokens(item_str);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            func_token_count += FUNC_END;
        }

        func_token_count
    }

    pub fn count_chat_tokens(
        &self,
        system_prompt: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> usize {
        let tokens_per_message = 4;
        let mut num_tokens = 0;

        if !system_prompt.is_empty() {
            num_tokens += self.count_tokens(system_prompt) + tokens_per_message;
        }

        for message in messages {
            if !message.metadata.agent_visible {
                continue;
            }
            num_tokens += tokens_per_message;
            for content in &message.content {
                if let Some(content_text) = content.as_text() {
                    num_tokens += self.count_tokens(content_text);
                } else if let Some(tool_request) = content.as_tool_request() {
                    if let Ok(tool_call) = tool_request.tool_call.as_ref() {
                        let text = format!(
                            "{}:{}:{:?}",
                            tool_request.id, tool_call.name, tool_call.arguments
                        );
                        num_tokens += self.count_tokens(&text);
                    }
                } else if let Some(tool_response_text) = content.as_tool_response_text() {
                    num_tokens += self.count_tokens(&tool_response_text);
                }
            }
        }

        if !tools.is_empty() {
            num_tokens += self.count_tokens_for_tools(tools);
        }

        num_tokens += 3; // Reply primer

        num_tokens
    }

    pub fn count_everything(
        &self,
        system_prompt: &str,
        messages: &[Message],
        tools: &[Tool],
        resources: &[String],
    ) -> usize {
        let mut num_tokens = self.count_chat_tokens(system_prompt, messages, tools);

        if !resources.is_empty() {
            for resource in resources {
                num_tokens += self.count_tokens(resource);
            }
        }
        num_tokens
    }

    pub fn clear_cache(&self) {
        self.token_cache.clear();
    }

    pub fn cache_size(&self) -> usize {
        self.token_cache.len()
    }
}

async fn get_tokenizer() -> Result<Arc<CoreBPE>, String> {
    let result = TOKENIZER
        .get_or_init(|| async {
            match tiktoken_rs::o200k_base() {
                Ok(bpe) => Ok(Arc::new(bpe)),
                Err(e) => Err(format!("Failed to initialize o200k_base tokenizer: {}", e)),
            }
        })
        .await;
    result.clone()
}

pub async fn create_token_counter() -> Result<TokenCounter, String> {
    TokenCounter::new().await
}

static SHARED_COUNTER: OnceCell<Result<Arc<TokenCounter>, String>> = OnceCell::const_new();

/// The process-wide [`TokenCounter`] (BR-56).
///
/// `create_token_counter` hands back a counter with an **empty** cache, so every
/// caller that builds one per call (the compaction check ran on every turn of
/// every session) re-encodes text it has already seen — the BPE work is real, and
/// the per-text memo is exactly what makes a repeated history cheap. One shared
/// counter means the cache survives across turns and sessions. The tokenizer
/// itself was already shared; only the memo was being thrown away.
pub async fn shared_token_counter() -> Result<Arc<TokenCounter>, String> {
    SHARED_COUNTER
        .get_or_init(|| async { TokenCounter::new().await.map(Arc::new) })
        .await
        .clone()
}

/// Conservative inflation applied to the o200k-based estimate for providers
/// whose real tokenizer Biorouter does not bundle. See `provider_token_calibration`.
const NON_OPENAI_TOKEN_MARGIN: f64 = 1.15;

/// Per-provider calibration factor for the cold-path token *estimate* (BR-15).
///
/// tiktoken's `o200k_base` is the only BPE compiled in, so it is used as a
/// universal approximation for every provider — a real Claude/Gemini/Llama
/// tokenizer would pull in a heavy dependency (SentencePiece / HuggingFace
/// vocab files), which the proposal explicitly defers. o200k is exact only for
/// OpenAI models; it *under-counts* the others (Claude/Gemini/Llama/Qwen
/// tokenizers are less token-efficient than GPT-4o's on English + code), so an
/// uncalibrated o200k estimate lets a first-turn / non-reporting-provider
/// session slip past the real context window without compacting.
///
/// This returns a multiplier applied only to the estimate. Erring high just
/// compacts slightly early (cheap); erring low silently overflows the window
/// (the costly dead-end this guards against), so non-OpenAI providers get a
/// small safe margin. The happy path uses the provider-reported token total and
/// never touches this.
///
/// Keyed on what the INSTANCE serves, not only on its registry name: a
/// provider whose inference runs on this machine (the built-in `ollama` and
/// `llamacpp`, and a declarative `ollama_compatible` provider registered under
/// any name) is serving an open-weights model through its own tokenizer, so
/// every one of them gets the same non-OpenAI margin — even one that happens to
/// be named `openai` or `azure`.
pub fn provider_token_calibration(provider: &dyn crate::providers::base::Provider) -> f64 {
    calibration_for(
        provider.get_name(),
        crate::providers::utils::runs_on_this_machine(provider),
    )
}

fn calibration_for(provider_name: &str, runs_on_this_machine: bool) -> f64 {
    if runs_on_this_machine {
        return NON_OPENAI_TOKEN_MARGIN;
    }
    match provider_name {
        // Native OpenAI tokenizer (o200k / cl100k) — the estimate is accurate.
        "openai" | "azure" => 1.0,
        // Everything else routes to a non-OpenAI tokenizer that o200k
        // under-counts. Aggregators (openrouter/litellm/databricks/…) can serve
        // GPT too, but the conservative margin only ever compacts a touch early.
        _ => NON_OPENAI_TOKEN_MARGIN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_token_caching() {
        let counter = create_token_counter().await.unwrap();

        let text = "This is a test for caching functionality";

        let count1 = counter.count_tokens(text);
        assert_eq!(counter.cache_size(), 1);

        let count2 = counter.count_tokens(text);
        assert_eq!(count1, count2);
        assert_eq!(counter.cache_size(), 1);

        let count3 = counter.count_tokens("Different text");
        assert_eq!(counter.cache_size(), 2);
        assert_ne!(count1, count3);
    }

    #[tokio::test]
    async fn test_cache_management() {
        let counter = create_token_counter().await.unwrap();

        counter.count_tokens("First text");
        counter.count_tokens("Second text");
        counter.count_tokens("Third text");

        assert_eq!(counter.cache_size(), 3);

        counter.clear_cache();
        assert_eq!(counter.cache_size(), 0);

        let count = counter.count_tokens("First text");
        assert!(count > 0);
        assert_eq!(counter.cache_size(), 1);
    }

    #[tokio::test]
    async fn test_concurrent_token_counter_creation() {
        let handles: Vec<_> = (0..10)
            .map(|_| tokio::spawn(async { create_token_counter().await.unwrap() }))
            .collect();

        let counters: Vec<_> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        let text = "Test concurrent creation";
        let expected_count = counters[0].count_tokens(text);

        for counter in &counters {
            assert_eq!(counter.count_tokens(text), expected_count);
        }
    }

    #[tokio::test]
    async fn test_cache_eviction_behavior() {
        let counter = create_token_counter().await.unwrap();

        let mut cached_texts = Vec::new();
        for i in 0..50 {
            let text = format!("Test string number {}", i);
            counter.count_tokens(&text);
            cached_texts.push(text);
        }

        assert!(counter.cache_size() <= MAX_TOKEN_CACHE_SIZE);

        let recent_text = &cached_texts[cached_texts.len() - 1];
        let start_size = counter.cache_size();

        counter.count_tokens(recent_text);
        assert_eq!(counter.cache_size(), start_size);
    }

    #[test]
    fn test_provider_token_calibration() {
        // OpenAI-family providers use the native o200k tokenizer: no inflation.
        assert_eq!(calibration_for("openai", false), 1.0);
        assert_eq!(calibration_for("azure", false), 1.0);

        // Non-OpenAI providers get a conservative safety margin (> 1.0) because
        // o200k under-counts their tokenizers.
        for name in ["anthropic", "google", "bedrock", "ollama", "openrouter"] {
            assert!(
                calibration_for(name, false) > 1.0,
                "expected a safety margin for {name}"
            );
        }
    }

    /// Every provider whose inference runs on this machine shares ONE
    /// calibration, whatever it is registered as: the built-in local providers,
    /// a declarative `ollama_compatible` provider under its own name, and even
    /// an `openai`-named provider repointed at a local server.
    #[test]
    fn local_providers_share_one_calibration_whatever_their_name() {
        let local: Vec<f64> = ["ollama", "llamacpp", "my_lab_ollama", "openai", "azure"]
            .into_iter()
            .map(|name| calibration_for(name, true))
            .collect();
        assert!(
            local.iter().all(|c| *c == NON_OPENAI_TOKEN_MARGIN),
            "{local:?}"
        );
    }

    /// Through a provider INSTANCE: the same registry name gets the OpenAI
    /// calibration off this machine and the shared local one on it, so the
    /// answer follows `affiliation()`, not `get_name()`.
    #[test]
    fn calibration_reads_the_provider_instance() {
        use crate::conversation::message::Message;
        use crate::model::ModelConfig;
        use crate::privacy::affiliation::ModelAffiliation;
        use crate::providers::base::{Provider, ProviderMetadata, ProviderUsage};
        use crate::providers::errors::ProviderError;
        use rmcp::model::Tool;

        /// Shaped like a self-hosted provider: Local and Private together while
        /// its host is loopback, neither once it is repointed elsewhere.
        struct NamedOpenAi(Option<ModelAffiliation>);

        #[async_trait::async_trait]
        impl Provider for NamedOpenAi {
            fn metadata() -> ProviderMetadata {
                ProviderMetadata::empty()
            }
            fn get_name(&self) -> &str {
                "openai"
            }
            fn tier(&self) -> crate::privacy::ProviderTier {
                match self.0 {
                    Some(_) => crate::privacy::ProviderTier::Private,
                    None => crate::privacy::ProviderTier::Public,
                }
            }
            fn affiliation(&self) -> Option<ModelAffiliation> {
                self.0
            }
            fn get_model_config(&self) -> ModelConfig {
                ModelConfig::new_or_fail("local-model")
            }
            async fn complete_with_model(
                &self,
                _model_config: &ModelConfig,
                _system: &str,
                _messages: &[Message],
                _tools: &[Tool],
            ) -> Result<(Message, ProviderUsage), ProviderError> {
                unreachable!("only asked for its calibration")
            }
        }

        assert_eq!(provider_token_calibration(&NamedOpenAi(None)), 1.0);
        assert_eq!(
            provider_token_calibration(&NamedOpenAi(Some(ModelAffiliation::Local))),
            NON_OPENAI_TOKEN_MARGIN
        );
    }

    #[tokio::test]
    async fn test_concurrent_cache_operations() {
        let counter = std::sync::Arc::new(create_token_counter().await.unwrap());

        let handles: Vec<_> = (0..20)
            .map(|i| {
                let counter_clone = counter.clone();
                tokio::spawn(async move {
                    let text = format!("Concurrent test {}", i % 5);
                    counter_clone.count_tokens(&text)
                })
            })
            .collect();

        let results: Vec<_> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        for result in results {
            assert!(result > 0);
        }

        assert!(counter.cache_size() > 0);
        assert!(counter.cache_size() <= MAX_TOKEN_CACHE_SIZE);
    }
}
