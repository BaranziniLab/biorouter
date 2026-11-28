use crate::knowledge::{
    convert::SourceInput,
    subagent::loop_::{Completer, SubAgent, SubAgentBounds, ToolDispatch},
    types::{Credibility, CredibilityTier},
};
use crate::safe_fetch::{FetchError, SafeFetch};
use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::{JsonObject, Tool};
use serde_json::Value;
use std::borrow::Cow;
use std::sync::Arc;
use tokio::time::Duration;

// ---------------------------------------------------------------------------
// System prompt
// ---------------------------------------------------------------------------

pub const AGENTIC_PROCEDURE: &str = concat!(
    "You are a biomedical source-credibility classifier.\n\n",
    "Given a source URL, title, or text, investigate its credibility using the available\n",
    "tools (fetch_url, crossref_search, openalex_search) and produce a structured assessment.\n\n",
    "Procedure:\n",
    "1. If the input looks like a URL, call fetch_url to retrieve a markdown excerpt.\n",
    "2. Look for DOIs, journal names, publisher names, or preprint server indicators in the text.\n",
    "3. If you find a plausible DOI or title, call crossref_search and/or openalex_search.\n",
    "4. Based on what you learn, decide the appropriate credibility tier:\n",
    "   - peer_reviewed : Published in a recognised peer-reviewed journal\n",
    "   - preprint      : Posted to a preprint server (arXiv, bioRxiv, medRxiv, …)\n",
    "   - book          : Academic book or book chapter\n",
    "   - gray_lit      : Government report, clinical guideline, white paper\n",
    "   - web           : General web page, blog, news article\n",
    "   - personal      : Personal notes or pasted text with no provenance\n",
    "5. When you have enough information, respond with ONLY a JSON object (no markdown fences,\n",
    "   no extra prose) matching exactly this schema:\n",
    "   {\n",
    "     \"tier\": \"<one of the tier names above>\",\n",
    "     \"confidence\": <float 0..1>,\n",
    "     \"publisher\": \"<string or null>\",\n",
    "     \"venue\": \"<string or null>\",\n",
    "     \"doi\": \"<string or null>\",\n",
    "     \"retracted\": <true|false>,\n",
    "     \"reasoning\": \"<1-2 sentences>\"\n",
    "   }\n\n",
    "Do not wrap the JSON in markdown code fences. Do not add any text before or after the JSON.\n",
    "If you cannot determine credibility, use tier=web, confidence=0.3.\n",
);

// ---------------------------------------------------------------------------
// Bounds
// ---------------------------------------------------------------------------

const MAX_STEPS: usize = 5;
const MAX_WALL_SECS: u64 = 30;
const MAX_TOKENS: u64 = 20_000;

// ---------------------------------------------------------------------------
// Tool schema helpers
// ---------------------------------------------------------------------------

/// Build an `Arc<JsonObject>` representing a JSON Schema "object" with the
/// given required properties (each typed "string").
fn make_schema(required: &[&str], optional: &[&str]) -> Arc<JsonObject> {
    let mut props = serde_json::Map::new();
    for name in required.iter().chain(optional.iter()) {
        props.insert(name.to_string(), serde_json::json!({ "type": "string" }));
    }
    let req_names: Vec<Value> = required
        .iter()
        .map(|n| Value::String(n.to_string()))
        .collect();
    let mut schema = serde_json::Map::new();
    schema.insert("type".into(), Value::String("object".into()));
    schema.insert("properties".into(), Value::Object(props));
    schema.insert("required".into(), Value::Array(req_names));
    Arc::new(schema)
}

fn agentic_tools() -> Vec<Tool> {
    vec![
        Tool::new(
            Cow::Borrowed("fetch_url"),
            Cow::Borrowed("Fetch a URL and return the first 4 KB as text."),
            make_schema(&["url"], &[]),
        ),
        Tool::new(
            Cow::Borrowed("crossref_search"),
            Cow::Borrowed(
                "Search Crossref for works matching a free-text query. Returns top-3 hits as JSON.",
            ),
            make_schema(&["query"], &[]),
        ),
        Tool::new(
            Cow::Borrowed("openalex_search"),
            Cow::Borrowed(
                "Search OpenAlex for works matching a free-text query. Returns top-3 hits as JSON.",
            ),
            make_schema(&["query"], &[]),
        ),
    ]
}

// ---------------------------------------------------------------------------
// AgenticToolDispatch
// ---------------------------------------------------------------------------

pub struct AgenticToolDispatch {
    /// For the two search tools, whose host is fixed here.
    client: reqwest::Client,
    /// For `fetch_url`, whose URL the classifier model chooses, so it reaches
    /// public internet addresses only (`crate::safe_fetch`).
    fetch: SafeFetch,
    crossref_base: String,
    openalex_base: String,
}

fn classifier_fetch() -> SafeFetch {
    SafeFetch::builder()
        .user_agent(CLASSIFIER_USER_AGENT)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("reqwest client")
}

const CLASSIFIER_USER_AGENT: &str = "Biorouter-Knowledge/1.0 (mailto:knowledge@biorouter.local)";

impl AgenticToolDispatch {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent(CLASSIFIER_USER_AGENT)
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("reqwest client"),
            fetch: classifier_fetch(),
            crossref_base: "https://api.crossref.org".to_string(),
            openalex_base: "https://api.openalex.org".to_string(),
        }
    }

    #[cfg(test)]
    pub fn with_bases(crossref_base: &str, openalex_base: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            fetch: classifier_fetch(),
            crossref_base: crossref_base.to_string(),
            openalex_base: openalex_base.to_string(),
        }
    }
}

impl Default for AgenticToolDispatch {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolDispatch for AgenticToolDispatch {
    async fn call(&self, name: &str, args: Value) -> Result<String> {
        match name {
            "fetch_url" => {
                let url = args["url"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("fetch_url: missing 'url'"))?;
                let resp = self
                    .fetch
                    .get(url)?
                    .send()
                    .await
                    .map_err(FetchError::from)?;
                let bytes = resp.bytes().await?;
                let text = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_string();
                Ok(strip_html_tags(&text))
            }

            "crossref_search" => {
                let query = args["query"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("crossref_search: missing 'query'"))?;
                let encoded: String =
                    url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
                let req_url = format!(
                    "{}/works?query={}&rows=3&select=DOI,title,publisher,type,container-title",
                    self.crossref_base, encoded
                );
                let resp = self.client.get(&req_url).send().await?;
                if !resp.status().is_success() {
                    return Ok("[]".to_string());
                }
                let body: Value = resp.json().await?;
                let items = body
                    .pointer("/message/items")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let hits: Vec<Value> = items
                    .into_iter()
                    .take(3)
                    .map(|item| {
                        serde_json::json!({
                            "doi": item.get("DOI").and_then(|v| v.as_str()).unwrap_or(""),
                            "title": item.get("title")
                                .and_then(|v| v.as_array())
                                .and_then(|a| a.first())
                                .and_then(|v| v.as_str())
                                .unwrap_or(""),
                            "publisher": item.get("publisher").and_then(|v| v.as_str()).unwrap_or(""),
                            "type": item.get("type").and_then(|v| v.as_str()).unwrap_or(""),
                        })
                    })
                    .collect();
                Ok(serde_json::to_string(&hits)?)
            }

            "openalex_search" => {
                let query = args["query"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("openalex_search: missing 'query'"))?;
                let encoded: String =
                    url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
                let req_url = format!(
                    "{}/works?search={}&per-page=3&select=doi,title,host_venue,type,is_retracted",
                    self.openalex_base, encoded
                );
                let resp = self.client.get(&req_url).send().await?;
                if !resp.status().is_success() {
                    return Ok("[]".to_string());
                }
                let body: Value = resp.json().await?;
                let results = body
                    .pointer("/results")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let hits: Vec<Value> = results
                    .into_iter()
                    .take(3)
                    .map(|item| {
                        serde_json::json!({
                            "doi": item.get("doi").and_then(|v| v.as_str()).unwrap_or(""),
                            "title": item.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                            "publisher": item.pointer("/host_venue/publisher")
                                .and_then(|v| v.as_str()).unwrap_or(""),
                            "type": item.get("type").and_then(|v| v.as_str()).unwrap_or(""),
                        })
                    })
                    .collect();
                Ok(serde_json::to_string(&hits)?)
            }

            other => anyhow::bail!("AgenticToolDispatch: unknown tool '{other}'"),
        }
    }
}

/// Crude HTML-tag stripper — enough to make fetched pages readable in a prompt.
fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Fallback helper
// ---------------------------------------------------------------------------

fn fallback(reason: &str) -> Credibility {
    Credibility {
        tier: CredibilityTier::Web,
        confidence: 0.3,
        publisher: None,
        venue: None,
        doi: None,
        retracted: false,
        reasoning: format!(
            "Agentic fallback could not parse JSON; final_text: {}",
            truncate(reason, 200)
        ),
        classifier_version: 2,
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

// ---------------------------------------------------------------------------
// Credibility JSON parsing
// ---------------------------------------------------------------------------

/// Try to parse a JSON blob (possibly embedded in prose) as a `Credibility`.
fn parse_credibility_json(text: &str) -> Option<Credibility> {
    // Try the whole text as pure JSON first.
    if let Ok(v) = serde_json::from_str::<Value>(text.trim()) {
        return from_value(v);
    }
    // Otherwise scan for the first `{` … `}` block.
    // SAFETY: `str::find('{')` returns a byte index that is always a valid
    // UTF-8 char boundary because `{` is an ASCII character (single byte).
    // Similarly, `char_indices()` returns byte offsets at char boundaries.
    #[allow(clippy::string_slice)]
    let start = text.find('{')?;
    #[allow(clippy::string_slice)]
    let rest = &text[start..];
    let mut depth = 0usize;
    let mut end = None;
    for (i, ch) in rest.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end?;
    #[allow(clippy::string_slice)]
    let candidate = &rest[..end];
    let v = serde_json::from_str::<Value>(candidate).ok()?;
    from_value(v)
}

fn from_value(v: Value) -> Option<Credibility> {
    let tier_str = v.get("tier")?.as_str()?;
    let tier = parse_tier(tier_str)?;
    let confidence = v
        .get("confidence")
        .and_then(|c| c.as_f64())
        .map(|f| f as f32)
        .unwrap_or(0.5);
    let reasoning = v
        .get("reasoning")
        .and_then(|r| r.as_str())
        .unwrap_or("Agentic classification.")
        .to_string();
    let publisher = v
        .get("publisher")
        .and_then(|p| p.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let venue = v
        .get("venue")
        .and_then(|p| p.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let doi = v
        .get("doi")
        .and_then(|p| p.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let retracted = v
        .get("retracted")
        .and_then(|r| r.as_bool())
        .unwrap_or(false);
    Some(Credibility {
        tier,
        confidence,
        publisher,
        venue,
        doi,
        retracted,
        reasoning,
        classifier_version: 2,
    })
}

fn parse_tier(s: &str) -> Option<CredibilityTier> {
    match s {
        "peer_reviewed" => Some(CredibilityTier::PeerReviewed),
        "preprint" => Some(CredibilityTier::Preprint),
        "book" => Some(CredibilityTier::Book),
        "gray_lit" => Some(CredibilityTier::GrayLit),
        "web" => Some(CredibilityTier::Web),
        "personal" => Some(CredibilityTier::Personal),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Source description helper
// ---------------------------------------------------------------------------

fn source_description(input: &SourceInput) -> String {
    match input {
        SourceInput::Url(u) => format!("Classify the credibility of this URL: {u}"),
        SourceInput::Text { text, title } => {
            let t = title.as_deref().unwrap_or("(no title)");
            let snippet = truncate(text, 300);
            format!("Classify the credibility of this pasted text.\nTitle: {t}\nSnippet: {snippet}")
        }
        SourceInput::File {
            filename, bytes, ..
        } => {
            let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_string();
            let snippet = truncate(&head, 300);
            format!("Classify the credibility of a file named '{filename}'.\nFirst 1 KB: {snippet}")
        }
        SourceInput::Path(path) => {
            format!(
                "Classify the credibility of a local file path named '{}'.",
                path.display()
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub async fn classify(input: &SourceInput, completer: Box<dyn Completer>) -> Result<Credibility> {
    classify_with_dispatch(
        input,
        completer,
        std::sync::Arc::new(AgenticToolDispatch::new()),
    )
    .await
}

/// Same as [`classify`], but with the tool dispatcher supplied by the caller.
///
/// Tests pass a dispatcher bound to a local mock: a sub-agent that decides to
/// call `crossref_search` otherwise reaches the live API, and the loop swallows
/// dispatch errors, so the network traffic is invisible until it hangs (issue
/// #55).
pub(crate) async fn classify_with_dispatch(
    input: &SourceInput,
    completer: Box<dyn Completer>,
    dispatch: std::sync::Arc<dyn ToolDispatch>,
) -> Result<Credibility> {
    let agent = SubAgent {
        completer,
        tools: agentic_tools(),
        system_prompt: AGENTIC_PROCEDURE.to_string(),
        bounds: SubAgentBounds {
            max_steps: MAX_STEPS,
            max_wall: Duration::from_secs(MAX_WALL_SECS),
            max_tokens: MAX_TOKENS,
        },
    };

    let user_msg = source_description(input);

    let result = agent.run(&user_msg, dispatch, None, None).await?;

    match parse_credibility_json(&result.final_text) {
        Some(c) => Ok(c),
        None => Ok(fallback(&result.final_text)),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::subagent::loop_::{LlmMessage, LlmReply, LlmToolCall};
    use async_trait::async_trait;
    use rmcp::model::Tool;
    use tokio::sync::Mutex;

    // ── MockCompleter ────────────────────────────────────────────────────────

    struct MockCompleter {
        replies: Mutex<Vec<LlmReply>>,
    }

    impl MockCompleter {
        fn new(replies: Vec<LlmReply>) -> Self {
            Self {
                replies: Mutex::new(replies),
            }
        }
    }

    #[async_trait]
    impl Completer for MockCompleter {
        async fn complete(
            &self,
            _system: &str,
            _messages: &[LlmMessage],
            _tools: &[Tool],
        ) -> Result<LlmReply> {
            let mut q = self.replies.lock().await;
            if q.is_empty() {
                // When queue drains (step budget hit), return empty text so loop exits.
                return Ok(LlmReply {
                    text: "".to_string(),
                    tool_calls: vec![],
                });
            }
            Ok(q.remove(0))
        }
    }

    fn text_reply(text: &str) -> LlmReply {
        LlmReply {
            text: text.to_string(),
            tool_calls: vec![],
        }
    }

    fn tool_call_reply(tool: &str) -> LlmReply {
        LlmReply {
            text: String::new(),
            tool_calls: vec![LlmToolCall {
                id: "req-1".into(),
                name: tool.to_string(),
                args: serde_json::json!({ "query": "test" }),
            }],
        }
    }

    fn valid_json_credibility() -> &'static str {
        r#"{
            "tier": "peer_reviewed",
            "confidence": 0.92,
            "publisher": "Springer Nature",
            "venue": "Nature Genetics",
            "doi": "10.1038/s41588-024-01893-6",
            "retracted": false,
            "reasoning": "Found in Crossref as a peer-reviewed journal article."
        }"#
    }

    // ── Test 1: happy-path JSON parse ────────────────────────────────────────

    #[tokio::test]
    async fn parses_valid_json_credibility() {
        let completer = MockCompleter::new(vec![text_reply(valid_json_credibility())]);
        let input = SourceInput::Url("https://example.com/paper".into());
        let c = classify(&input, Box::new(completer)).await.unwrap();
        assert_eq!(c.tier, CredibilityTier::PeerReviewed);
        assert!((c.confidence - 0.92).abs() < 0.01);
        assert_eq!(c.publisher.as_deref(), Some("Springer Nature"));
        assert_eq!(c.venue.as_deref(), Some("Nature Genetics"));
        assert_eq!(c.doi.as_deref(), Some("10.1038/s41588-024-01893-6"));
        assert!(!c.retracted);
        assert_eq!(c.classifier_version, 2);
    }

    // ── Test 2: fallback when parse fails ────────────────────────────────────

    #[tokio::test]
    async fn falls_back_when_parse_fails() {
        let completer = MockCompleter::new(vec![text_reply("I don't know")]);
        let input = SourceInput::Url("https://example.com/unknown".into());
        let c = classify(&input, Box::new(completer)).await.unwrap();
        assert_eq!(c.tier, CredibilityTier::Web);
        assert!((c.confidence - 0.3).abs() < 0.01);
        assert!(
            c.reasoning.contains("I don't know"),
            "reasoning should mention final_text, got: {}",
            c.reasoning
        );
        assert_eq!(c.classifier_version, 2);
    }

    // ── Test 3: step budget respected ────────────────────────────────────────

    #[tokio::test]
    async fn respects_step_budget() {
        use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

        // Supply 20 tool-call replies; budget is MAX_STEPS=5, so it stops early.
        //
        // Those replies are real `crossref_search` calls, and the sub-agent loop
        // swallows dispatch errors — so with the production dispatcher this test
        // silently made live requests to api.crossref.org and still passed
        // (issue #55). Bind it to a local mock instead.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": { "items": [] }
            })))
            .mount(&server)
            .await;
        let dispatch = AgenticToolDispatch::with_bases(&server.uri(), &server.uri());

        let replies: Vec<LlmReply> = (0..20)
            .map(|_| tool_call_reply("crossref_search"))
            .collect();
        let completer = MockCompleter::new(replies);
        let input = SourceInput::Url("https://example.com/paper".into());
        let c = classify_with_dispatch(&input, Box::new(completer), std::sync::Arc::new(dispatch))
            .await
            .unwrap();
        // Step budget kicks in → fallback Web tier.
        assert_eq!(c.tier, CredibilityTier::Web);
        assert_eq!(c.classifier_version, 2);
        // And every search went to the mock, not to the live API.
        assert!(!server.received_requests().await.unwrap().is_empty());
    }

    /// The classifier sub-agent picks the URL its `fetch_url` tool reads, so the
    /// tool reaches public addresses only.
    #[tokio::test]
    async fn fetch_url_tool_refuses_a_loopback_address() {
        use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("local secret"))
            .expect(0)
            .mount(&server)
            .await;
        let dispatch = AgenticToolDispatch::new();
        let error = match dispatch
            .call("fetch_url", serde_json::json!({ "url": server.uri() }))
            .await
        {
            Ok(text) => panic!("a loopback URL must be refused, got: {text}"),
            Err(error) => format!("{error:#}"),
        };
        assert!(error.contains("loopback"), "{error}");
    }

    // ── Test 4: parse_credibility_json edge cases ─────────────────────────────

    #[test]
    fn parse_extracts_json_from_prose() {
        let text = "Here is my assessment:\n{\"tier\":\"preprint\",\"confidence\":0.8,\"reasoning\":\"arXiv URL.\",\"retracted\":false}";
        let c = parse_credibility_json(text).unwrap();
        assert_eq!(c.tier, CredibilityTier::Preprint);
    }

    #[test]
    fn parse_returns_none_for_invalid_tier() {
        let text = r#"{"tier":"unknown_tier","confidence":0.5,"reasoning":"x","retracted":false}"#;
        assert!(parse_credibility_json(text).is_none());
    }

    // ── Test 5: AgenticToolDispatch crossref_search with wiremock ────────────

    #[tokio::test]
    async fn agentic_tool_dispatch_calls_crossref() {
        use wiremock::{
            matchers::{method, path, query_param_contains},
            Mock, MockServer, ResponseTemplate,
        };

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/works"))
            .and(query_param_contains("query", "nature"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": {
                    "items": [
                        {
                            "DOI": "10.1038/s41588",
                            "title": ["A great paper"],
                            "publisher": "Springer Nature",
                            "type": "journal-article"
                        }
                    ]
                }
            })))
            .mount(&server)
            .await;

        let dispatch = AgenticToolDispatch::with_bases(&server.uri(), "https://api.openalex.org");
        let result = dispatch
            .call(
                "crossref_search",
                serde_json::json!({ "query": "nature genetics" }),
            )
            .await
            .unwrap();

        let hits: Vec<Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["doi"].as_str(), Some("10.1038/s41588"));
        assert_eq!(hits[0]["publisher"].as_str(), Some("Springer Nature"));
    }
}
