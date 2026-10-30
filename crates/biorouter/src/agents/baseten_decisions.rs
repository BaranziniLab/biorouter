use std::sync::Arc;
use std::time::Duration;

use anyhow::{ensure, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::ToSchema;

use super::exit_criteria::{DecisionType, ExitCriteria};
use crate::config::Config;
use crate::conversation::message::Message;
use crate::model::ModelConfig;
use crate::providers::api_client::{ApiClient, AuthMethod};
use crate::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use crate::providers::errors::ProviderError;
use rmcp::model::Tool;

// System One is a separate protocol; these models must not enter the chat-model catalog.
// Contract: docs.baseten.co/reference/inference-api/system-one
// Inventory: docs.baseten.co/inference/model-apis/decision-models#supported-models
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct BasetenDecisionModel {
    pub name: String,
    pub slug: String,
}

pub fn models() -> Vec<BasetenDecisionModel> {
    [
        ("elephant-1", "decision-models/baseten-elephant-9b-latest"),
        ("mammoth-1", "decision-models/baseten-mammoth-27b-latest"),
        ("joey-1", "decision-models/baseten-joey-2b-latest"),
    ]
    .into_iter()
    .map(|(name, slug)| BasetenDecisionModel {
        name: name.into(),
        slug: slug.into(),
    })
    .collect()
}

pub fn model_slug(name: &str) -> Option<String> {
    models()
        .into_iter()
        .find(|m| m.name == name || m.slug == name)
        .map(|m| m.slug)
}

pub fn uses_system_one(criteria: &ExitCriteria) -> bool {
    criteria.provider == "baseten" && model_slug(&criteria.model).is_some()
}

pub async fn create_provider(criteria: &ExitCriteria) -> Result<Arc<dyn Provider>> {
    if uses_system_one(criteria) {
        let key: String = Config::global().get_secret("BASETEN_API_KEY")?;
        let client = ApiClient::with_timeout(
            "https://inference.baseten.co".into(),
            AuthMethod::BearerToken(key),
            Duration::from_secs(30),
        )?;
        Ok(Arc::new(BasetenDecisionProvider::new(
            criteria.clone(),
            client,
        )?))
    } else {
        crate::providers::create_with_named_model(&criteria.provider, &criteria.model).await
    }
}

pub struct BasetenDecisionProvider {
    criteria: ExitCriteria,
    model: ModelConfig,
    client: ApiClient,
}

impl BasetenDecisionProvider {
    pub fn new(criteria: ExitCriteria, client: ApiClient) -> Result<Self> {
        criteria.validate()?;
        let slug = model_slug(&criteria.model)
            .ok_or_else(|| anyhow::anyhow!("Unknown Baseten decision model"))?;
        let provider = Self {
            criteria,
            model: ModelConfig::new(&slug)?,
            client,
        };
        provider.question()?;
        Ok(provider)
    }

    pub fn question(&self) -> Result<Value> {
        let mut question = json!({"instructions": self.criteria.prompt});
        match &self.criteria.decision {
            DecisionType::Noul { .. } | DecisionType::Boolean { .. } => {
                question["type"] = json!("noul")
            }
            DecisionType::Choice { choices, .. } => {
                question["type"] = json!("choice");
                question["criteria"] = choices
                    .iter()
                    .map(|c| (c.clone(), json!(c)))
                    .collect::<serde_json::Map<_, _>>()
                    .into();
            }
            DecisionType::Ranking { levels, .. } => {
                ensure!(
                    (2..=10).contains(&levels.len()),
                    "Baseten ranking requires 2–10 ordered rubric levels, starting at zero"
                );
                question["type"] = json!("score");
                question["criteria"] = json!(levels);
            }
        }
        Ok(question)
    }

    fn verdict(&self, answer: &Value) -> Value {
        let expected = match self.criteria.decision {
            DecisionType::Noul { .. } | DecisionType::Boolean { .. } => "noul",
            DecisionType::Choice { .. } => "choice",
            DecisionType::Ranking { .. } => "score",
        };
        let value = if answer["type"].as_str() != Some(expected) {
            Value::Null
        } else {
            match self.criteria.decision {
                DecisionType::Boolean { .. } => answer["noul"]
                    .as_f64()
                    .filter(|p| p.is_finite() && (0.0..=1.0).contains(p))
                    .map(|p| json!(p >= 0.5))
                    .unwrap_or(Value::Null),
                _ => answer[expected].clone(),
            }
        };
        json!({"value": value, "reason": format!("Baseten System One returned {expected}: {}. This is a typed decision, not a generated explanation.", answer[expected])})
    }
}

#[async_trait]
impl Provider for BasetenDecisionProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new(
            "baseten",
            "Baseten decision models",
            "System One typed decisions",
            "decision-models/baseten-elephant-9b-latest",
            vec![],
            "",
            vec![],
        )
    }

    fn get_name(&self) -> &str {
        "baseten"
    }

    fn get_model_config(&self) -> ModelConfig {
        self.model.clone()
    }

    async fn complete_with_model(
        &self,
        _: &ModelConfig,
        _: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        if !tools.is_empty() {
            return Err(ProviderError::ExecutionError(
                "Decision models cannot call tools".into(),
            ));
        }
        let text = messages
            .last()
            .ok_or_else(|| ProviderError::ExecutionError("Decision evidence is missing".into()))?
            .as_concat_text();
        let state: Value = serde_json::from_str(&text).unwrap_or_else(|_| json!(text));
        let body = json!({"model": self.model.model_name, "state": state, "questions": {"loop_exit": self.question()?}});
        let response = self.client.response_post("/v1/systemone", &body).await?;
        let status = response.status();
        if !status.is_success() {
            // Do not echo a remote error body: validation errors can contain the submitted state.
            return Err(ProviderError::RequestFailed(format!(
                "Baseten System One returned HTTP {status}"
            )));
        }
        let response: Value = response.json().await?;
        let usage = Usage {
            input_tokens: response["usage"]["input_tokens"]
                .as_i64()
                .and_then(|n| i32::try_from(n).ok()),
            output_tokens: response["usage"]["output_tokens"]
                .as_i64()
                .and_then(|n| i32::try_from(n).ok()),
            ..Usage::default()
        };
        let mut usage = ProviderUsage::new(self.model.model_name.clone(), usage);
        usage.usage.total_tokens = usage
            .usage
            .input_tokens
            .zip(usage.usage.output_tokens)
            .and_then(|(a, b)| a.checked_add(b));
        usage.provider = Some("baseten".into());
        Ok((
            Message::assistant()
                .with_text(self.verdict(&response["answers"]["loop_exit"]).to_string()),
            usage,
        ))
    }
}
