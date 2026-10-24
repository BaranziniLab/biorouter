use crate::mcp_utils::ToolResult;
use crate::providers::base::Provider;
use rmcp::model::{CallToolResult, Tool};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use utoipa::ToSchema;

/// Type alias for the tool result channel receiver
pub type ToolResultReceiver = Arc<Mutex<mpsc::Receiver<(String, ToolResult<CallToolResult>)>>>;

// We use double Arc here to allow easy provider swaps while sharing concurrent access
pub type SharedProvider = Arc<Mutex<Option<Arc<dyn Provider>>>>;

/// Default timeout for retry operations (5 minutes)
pub const DEFAULT_RETRY_TIMEOUT_SECONDS: u64 = 300;

/// Default timeout for on_failure operations (10 minutes - longer for on_failure tasks)
pub const DEFAULT_ON_FAILURE_TIMEOUT_SECONDS: u64 = 600;

/// Configuration for retry logic in workflow execution
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RetryConfig {
    /// Maximum number of retry attempts before giving up
    pub max_retries: u32,
    /// List of success checks to validate workflow completion
    pub checks: Vec<SuccessCheck>,
    /// Optional shell command to run on failure for cleanup
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_failure: Option<String>,
    /// Timeout in seconds for individual shell commands (default: 300 seconds)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    /// Timeout in seconds for on_failure commands (default: 600 seconds)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_failure_timeout_seconds: Option<u64>,
}

impl RetryConfig {
    /// Validates the retry configuration values
    pub fn validate(&self) -> Result<(), String> {
        if self.max_retries == 0 {
            return Err("max_retries must be greater than 0".to_string());
        }

        if let Some(timeout) = self.timeout_seconds {
            if timeout == 0 {
                return Err("timeout_seconds must be greater than 0 if specified".to_string());
            }
        }

        if let Some(on_failure_timeout) = self.on_failure_timeout_seconds {
            if on_failure_timeout == 0 {
                return Err(
                    "on_failure_timeout_seconds must be greater than 0 if specified".to_string(),
                );
            }
        }

        Ok(())
    }
}

/// A single success check to validate workflow completion, or (BR-48) to gate
/// an interactive chat turn from finishing with unmet conditions.
///
/// Originally a single `Shell` variant used only by the workflow retry path.
/// BR-48 added the non-shell variants and a done-ness gate so an interactive
/// session can also declare "not done until these hold" — see
/// [`crate::agents::done_gate`].
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type")]
pub enum SuccessCheck {
    /// Execute a shell command and check its exit status (0 = pass).
    #[serde(alias = "shell")]
    Shell {
        /// The shell command to execute
        command: String,
    },
    /// BR-48: pass when a file exists at `path`. A relative `path` resolves
    /// against the session working directory (interactive gate) or the process
    /// working directory (workflow retry).
    #[serde(alias = "file_exists", alias = "fileexists")]
    FileExists {
        /// The file (or directory) that must exist.
        path: String,
    },
    /// BR-48: run `command` and pass when its combined stdout+stderr contains
    /// `substring`. Unlike [`Shell`](SuccessCheck::Shell), the exit status is
    /// ignored — this checks the *content* of the output (e.g. `0 failed`,
    /// `PASS`, a coverage percentage).
    #[serde(alias = "output_contains", alias = "outputcontains")]
    OutputContains {
        /// The shell command whose output is inspected.
        command: String,
        /// The substring the output must contain.
        substring: String,
    },
    /// BR-48: pass when the JSON file at `path` parses and validates against the
    /// inline JSON Schema `schema` (same engine as the workflow final-output
    /// contract, [`crate::agents::final_output_tool`]).
    #[serde(alias = "json_schema", alias = "jsonschema")]
    JsonSchema {
        /// The JSON file to validate.
        path: String,
        /// The JSON Schema the file's contents must satisfy.
        schema: serde_json::Value,
    },
}

/// A frontend tool that will be executed by the frontend rather than an extension
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendTool {
    pub name: String,
    pub tool: Tool,
}

/// Session configuration for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// Identifier of the underlying Session
    pub id: String,
    /// ID of the schedule that triggered this session, if any
    pub schedule_id: Option<String>,
    /// Maximum turns without user input. Zero is unlimited; None inherits configuration.
    pub max_turns: Option<u32>,
    /// Maximum tool calls per reply. Zero is unlimited; None inherits configuration.
    #[serde(default)]
    pub max_tool_calls: Option<u32>,
    /// Per-reply wall-clock / token / dollar ceiling (BR-35). Bounds how long a
    /// reply runs and what it costs, which the iteration caps above do not:
    /// 429 backoff compounds *inside* an iteration, and one iteration can pull a
    /// whole 200k-token context. Unset axes fall back to the global config keys
    /// (`BIOROUTER_REPLY_BUDGET_*`); unset everywhere means unbounded, as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<crate::agents::budget::ReplyBudget>,
    /// Retry configuration for automated validation and recovery
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_config: Option<RetryConfig>,
    /// BR-63: how hard to think on this turn (quick / normal / deep). `None`
    /// falls back to the session's sticky `/effort` setting, and then to
    /// `Normal` — which is a strict no-op, so an unset effort behaves exactly
    /// as before. See [`crate::agents::effort`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<crate::agents::effort::ReasoningEffort>,
}
