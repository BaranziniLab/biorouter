#[derive(Debug, Clone)]
pub struct ProviderScriptOutput {
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr_tail: String,
}
