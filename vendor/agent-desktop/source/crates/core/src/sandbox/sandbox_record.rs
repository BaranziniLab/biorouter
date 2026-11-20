use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SandboxRecord {
    pub(crate) name: String,
    pub(crate) provider: String,
    pub(crate) created_at: u64,
}
