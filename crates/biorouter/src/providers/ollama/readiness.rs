use super::{OllamaProvider, ProviderError};

const READINESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
use crate::providers::errors::sanitized_endpoint;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct OllamaInstalledModel {
    pub name: String,
    pub model: Option<String>,
    pub size: Option<u64>,
    pub digest: Option<String>,
    pub modified_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct OllamaReadiness {
    pub base_url: String,
    pub reachable: bool,
    pub version: Option<String>,
    pub models: Vec<OllamaInstalledModel>,
    pub selected_model: Option<String>,
    pub model_available: Option<bool>,
    pub context_window: Option<usize>,
    pub supports_tools: Option<bool>,
    pub error: Option<String>,
    pub phase: String,
}

fn model_matches(installed: &str, selected: &str) -> bool {
    installed == selected
        || (!selected
            .rsplit('/')
            .next()
            .unwrap_or(selected)
            .contains(':')
            && installed == format!("{selected}:latest"))
}

impl OllamaProvider {
    fn service_recovery(&self) -> &'static str {
        if crate::providers::api_client::is_loopback_host(&self.resolved_base_url) {
            "Open or start Ollama on the computer running Biorouter, then check again. \
             The host setting chooses a client address; it does not start Ollama."
        } else {
            "Check the Ollama service and network access on that server, then check again."
        }
    }

    pub(super) fn service_error(&self, error: ProviderError) -> ProviderError {
        if error.kind() != crate::providers::errors::ProviderErrorKind::Network {
            return error;
        }
        let endpoint = url::Url::parse(&self.resolved_base_url)
            .map(|url| sanitized_endpoint(&url))
            .unwrap_or_else(|_| "the configured endpoint".to_string());
        let recovery = self.service_recovery();
        match error {
            ProviderError::Transport(diagnostic) => {
                ProviderError::Transport(Box::new((*diagnostic).for_service(
                    "Ollama",
                    recovery,
                    crate::providers::api_client::is_loopback_host(&self.resolved_base_url),
                )))
            }
            error => ProviderError::RequestFailed(format!(
                "Cannot reach Ollama at {endpoint}. {recovery} Connection diagnostics: {error}"
            )),
        }
    }

    async fn probe_service(&self) -> anyhow::Result<(String, Vec<OllamaInstalledModel>)> {
        let version = self.api_client.response_get("api/version").await?;
        if !version.status().is_success() {
            anyhow::bail!("Ollama version check returned HTTP {}", version.status());
        }
        let version = version.json::<serde_json::Value>().await?;
        let version = version
            .get("version")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Ollama did not return a server version"))?
            .to_string();
        let tags = self.api_client.response_get("api/tags").await?;
        if !tags.status().is_success() {
            anyhow::bail!("Ollama model listing returned HTTP {}", tags.status());
        }
        let tags = tags.json::<serde_json::Value>().await?;
        let models = tags
            .get("models")
            .and_then(|value| value.as_array())
            .ok_or_else(|| anyhow::anyhow!("Ollama did not return an installed-model list"))?
            .iter()
            .filter_map(|value| {
                Some(OllamaInstalledModel {
                    name: value
                        .get("name")
                        .or_else(|| value.get("model"))?
                        .as_str()?
                        .to_string(),
                    model: value
                        .get("model")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    size: value.get("size").and_then(|v| v.as_u64()),
                    digest: value
                        .get("digest")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    modified_at: value
                        .get("modified_at")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                })
            })
            .collect::<Vec<_>>();
        Ok((version, models))
    }

    /// A bounded, prompt-free probe from the same daemon and HTTP client as
    /// inference. Service reachability does not claim that inference succeeded.
    pub async fn readiness(&self, model: Option<&str>) -> OllamaReadiness {
        let mut status = OllamaReadiness {
            base_url: url::Url::parse(&self.resolved_base_url)
                .map(|url| sanitized_endpoint(&url))
                .unwrap_or_else(|_| "the configured endpoint".to_string()),
            reachable: false,
            version: None,
            models: Vec::new(),
            selected_model: model.map(str::to_string),
            model_available: None,
            context_window: None,
            supports_tools: None,
            error: None,
            phase: "service".to_string(),
        };
        match tokio::time::timeout(READINESS_TIMEOUT, self.probe_service()).await {
            Ok(Ok((version, models))) => {
                status.reachable = true;
                status.version = Some(version);
                status.models = models;
                status.phase = "ready".to_string();
            }
            Ok(Err(error)) => {
                status.error = Some(format!(
                    "Ollama readiness check at {} failed: {}. Check the Ollama service, then check again.",
                    status.base_url,
                    self.service_error(error.into())
                ));
                return status;
            }
            Err(_) => {
                status.error = Some(format!(
                    "Cannot reach Ollama at {}: readiness check timed out after {} seconds. \
                     {} No inference was attempted.",
                    status.base_url,
                    READINESS_TIMEOUT.as_secs(),
                    self.service_recovery()
                ));
                return status;
            }
        }
        if let Some(model) = model {
            let available = status.models.iter().any(|installed| {
                model_matches(&installed.name, model)
                    || installed
                        .model
                        .as_deref()
                        .is_some_and(|name| model_matches(name, model))
            });
            status.model_available = Some(available);
            if !available {
                status.phase = "model".to_string();
                status.error = Some(format!(
                    "Ollama is reachable at {}, but model '{model}' is not installed. \
                     Pull or choose an installed model, then check again.",
                    status.base_url
                ));
                return status;
            }
            let details = async {
                let facts = self.refresh_model_facts(model).await;
                let context_window = if facts.is_some() {
                    self.context_window_for(model).await
                } else {
                    None
                };
                (
                    facts.and_then(|facts| facts.supports_tools()),
                    context_window,
                )
            };
            if let Ok((supports_tools, context_window)) =
                tokio::time::timeout(READINESS_TIMEOUT, details).await
            {
                status.supports_tools = supports_tools;
                status.context_window = context_window;
                if supports_tools == Some(false) {
                    status.phase = "model".to_string();
                    status.error = Some(format!(
                        "Ollama model '{model}' is installed, but does not support the tools \
                         Biorouter needs. Choose a model with tool support."
                    ));
                }
            }
        }
        status
    }
}
