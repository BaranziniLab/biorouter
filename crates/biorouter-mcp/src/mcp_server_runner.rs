use std::str::FromStr;

use anyhow::Result;
use rmcp::{transport::stdio, ServiceExt};

#[derive(Clone, Debug)]
pub enum McpCommand {
    AutoVisualiser,
    ComputerController,
    WebDocuments,
    Developer,
    Memory,
}

impl FromStr for McpCommand {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace(' ', "").as_str() {
            "autovisualiser" => Ok(McpCommand::AutoVisualiser),
            "computercontroller" => Ok(McpCommand::ComputerController),
            "webdocuments" => Ok(McpCommand::WebDocuments),
            "developer" => Ok(McpCommand::Developer),
            "memory" => Ok(McpCommand::Memory),
            _ => Err(format!("Invalid command: {}", s)),
        }
    }
}

impl McpCommand {
    pub fn name(&self) -> &str {
        match self {
            McpCommand::AutoVisualiser => "autovisualiser",
            McpCommand::ComputerController => "computercontroller",
            McpCommand::WebDocuments => "webdocuments",
            McpCommand::Developer => "developer",
            McpCommand::Memory => "memory",
        }
    }
}

pub async fn serve<S>(server: S) -> Result<()>
where
    S: rmcp::ServerHandler,
{
    let service = server.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("serving error: {:?}", e);
    })?;

    service.waiting().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_tutorial_is_not_registered_or_runnable() {
        assert!(McpCommand::from_str("tutorial").is_err());
        assert!(!crate::BUILTIN_EXTENSIONS.contains_key("tutorial"));
    }
}
