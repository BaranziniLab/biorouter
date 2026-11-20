use serde_json::Value;

use crate::sandbox::sandbox_state::SandboxState;

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderReply {
    pub(crate) state: SandboxState,
    pub(crate) message: String,
    pub(crate) next: Option<String>,
    pub(crate) data: Option<Value>,
}
