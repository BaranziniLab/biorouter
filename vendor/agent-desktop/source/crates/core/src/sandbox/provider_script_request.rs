use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ProviderScriptRequest {
    pub script: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub working_dir: PathBuf,
    pub timeout: Duration,
}
