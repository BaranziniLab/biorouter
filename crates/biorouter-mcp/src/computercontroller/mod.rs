use crate::computer_use::{contract, SessionRuntime};
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, ErrorData, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    },
    service::RequestContext,
    RoleServer, ServerHandler,
};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone, Default)]
pub struct ComputerControllerServer {
    runtime: Arc<Mutex<SessionRuntime>>,
}

impl ComputerControllerServer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn tools() -> Vec<Tool> {
        contract::tools()
    }
}

/// The grant the host admitted this call with (AD-10). A call that names no grant is treated as
/// look only, so a missing field can never widen what a call may do.
fn admitted_grant(meta: &rmcp::model::Meta) -> contract::Grant {
    meta.get("computer_use_grant")
        .and_then(Value::as_str)
        .and_then(contract::Grant::parse)
        .unwrap_or(contract::Grant::Observe)
}

impl ServerHandler for ComputerControllerServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: "biorouter-computercontroller".into(),
                title: Some("Biorouter Copilot".into()),
                version: env!("CARGO_PKG_VERSION").into(),
                icons: None,
                website_url: None,
            },
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(contract::instructions()),
            ..Default::default()
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: Self::tools(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let tool = Self::tools()
            .into_iter()
            .find(|tool| tool.name == request.name)
            .ok_or_else(|| ErrorData::invalid_params("Unknown Biorouter Copilot tool", None))?;
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let schema = Value::Object((*tool.input_schema).clone());
        let validator = jsonschema::validator_for(&schema)
            .map_err(|_| ErrorData::internal_error("Invalid Biorouter Copilot contract", None))?;
        if !validator.is_valid(&arguments) {
            let retired = [
                "element_index",
                "click_method",
                "window_title",
                "text_limit_nodes",
            ]
            .iter()
            .any(|key| arguments.get(*key).is_some());
            return Err(ErrorData::invalid_params(
                if retired {
                    "Arguments do not match the Biorouter Copilot tool schema: elements are now named by reference (such as @s1abc:e4) from get_app_state, not by element_index or window_title."
                } else {
                    "Arguments do not match the Biorouter Copilot tool schema"
                },
                None,
            ));
        }
        let session = context
            .meta
            .get("biorouter-session-id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                ErrorData::invalid_params(
                    "computer_use_approval_required: missing chat identity",
                    None,
                )
            })?;
        let generation = context
            .meta
            .get("computer_use_generation")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                ErrorData::invalid_params(
                    "computer_use_approval_required: missing approved task generation",
                    None,
                )
            })?;
        let grant = admitted_grant(&context.meta);
        let mut runtime = tokio::select! {
            biased;
            _ = context.ct.cancelled() => return Err(ErrorData::internal_error("computer_use_cancelled", None)),
            runtime = self.runtime.lock() => runtime,
        };
        runtime.ensure_session(session)?;
        crate::computer_use::register_session(
            session,
            generation,
            &self.runtime,
            context.ct.clone(),
        );
        runtime
            .call(
                session,
                generation,
                &request.name,
                arguments,
                grant,
                context.ct.clone(),
            )
            .await
    }
}
