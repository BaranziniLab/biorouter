use crate::agents::tool_errors::{ToolError, ToolErrorKind};
use crate::mcp_utils::ToolResult;
use rmcp::model::CallToolResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const OPERATION_META_KEY: &str = "io.biorouter/operation";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationError {
    pub code: String,
    pub kind: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationOutcome {
    pub version: u32,
    pub id: String,
    pub status: OperationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<OperationError>,
}

impl OperationOutcome {
    pub fn from_value(value: &Value) -> Option<Self> {
        if value.get("error").is_some_and(Value::is_null)
            || value
                .get("error")
                .and_then(|error| error.get("phase"))
                .is_some_and(Value::is_null)
        {
            return None;
        }
        let outcome: Self = serde_json::from_value(value.clone()).ok()?;
        if outcome.version != 1 || outcome.id.trim().is_empty() {
            return None;
        }
        if outcome.error.as_ref().is_some_and(|error| {
            [&error.code, &error.kind, &error.message]
                .iter()
                .any(|text| text.trim().is_empty())
                || error
                    .phase
                    .as_ref()
                    .is_some_and(|phase| phase.trim().is_empty())
        }) {
            return None;
        }
        Some(outcome)
    }

    pub fn from_call(call: &CallToolResult) -> Option<Self> {
        Self::from_value(call.meta.as_ref()?.get(OPERATION_META_KEY)?)
    }

    pub fn from_result(result: &ToolResult<CallToolResult>) -> Option<Self> {
        result.as_ref().ok().and_then(Self::from_call)
    }

    pub fn failure(&self) -> Option<ToolError> {
        if self.status != OperationStatus::Failed {
            return None;
        }
        let (kind, message, retryable) = self.error.as_ref().map_or_else(
            || {
                (
                    ToolErrorKind::ToolFailure,
                    "No operation error was supplied.".to_string(),
                    false,
                )
            },
            |error| {
                let kind =
                    serde_json::from_value::<ToolErrorKind>(Value::String(error.kind.clone()))
                        .unwrap_or(ToolErrorKind::ToolFailure);
                (
                    kind,
                    format!("{}: {}", error.code, error.message),
                    error.retryable,
                )
            },
        );
        let mut error = ToolError::new(kind, format!("Operation {} failed: {message}", self.id));
        error.retryable = retryable;
        Some(error)
    }
}

pub fn failure(tool_name: &str, result: &ToolResult<CallToolResult>) -> Option<ToolError> {
    observed_operations(tool_name, result)
        .into_iter()
        .find_map(|(_, operation)| operation.failure())
}

pub fn observed_operations(
    tool_name: &str,
    result: &ToolResult<CallToolResult>,
) -> Vec<(String, OperationOutcome)> {
    let Ok(call) = result else {
        return Vec::new();
    };
    let mut operations = Vec::new();
    if let Some(operation) = OperationOutcome::from_call(call) {
        operations.push((tool_name.to_string(), operation));
    }
    if !super::code_execution_extension::is_execute_code_call(tool_name) {
        return operations;
    }
    if let Some(records) = call
        .meta
        .as_ref()
        .and_then(|meta| meta.get("biorouter/tool-calls"))
        .and_then(Value::as_array)
    {
        for record in records {
            let Some(tool) = record
                .get("tool")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
            else {
                continue;
            };
            if let Some(operation) = record
                .get("operation")
                .and_then(OperationOutcome::from_value)
            {
                operations.push((tool.to_string(), operation));
            }
        }
    }
    operations
}

pub fn extension_name(tool_name: &str) -> &str {
    tool_name
        .split_once("__")
        .map_or(tool_name, |(extension, _)| extension)
}
