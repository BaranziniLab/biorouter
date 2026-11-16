use std::str::FromStr;

use anyhow::Result;
use rmcp::{transport::stdio, ServiceExt};

/// The built-in servers `biorouter mcp <SERVER>` runs as a subprocess.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpCommand {
    AutoVisualiser,
    ComputerController,
    WebDocuments,
    Developer,
    Memory,
}

impl FromStr for McpCommand {
    type Err = String;

    /// The server `s` names, ignoring case and spaces. A name that is none of them is refused
    /// with the names that work (W1-13), where it said only "Invalid command".
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let wanted = s.to_lowercase().replace(' ', "");
        Self::ALL
            .into_iter()
            .find(|server| server.name() == wanted)
            .ok_or_else(|| {
                format!(
                    "Unknown server '{s}'. Choose one of: {}.",
                    Self::names().join(", ")
                )
            })
    }
}

impl McpCommand {
    /// Every server, in the order `biorouter mcp --help` lists them: the one list the parser,
    /// its refusal and the help all read.
    pub const ALL: [McpCommand; 5] = [
        McpCommand::AutoVisualiser,
        McpCommand::ComputerController,
        McpCommand::WebDocuments,
        McpCommand::Developer,
        McpCommand::Memory,
    ];

    /// The name each server in [`Self::ALL`] is run by.
    pub fn names() -> [&'static str; 5] {
        Self::ALL.map(|server| server.name())
    }

    pub fn name(&self) -> &'static str {
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

    /// W1-13: an unknown name is answered with the five names that work, not a bare
    /// "Invalid command".
    #[test]
    fn an_unknown_name_lists_the_five_servers() {
        assert_eq!(
            McpCommand::from_str("nosuch").unwrap_err(),
            "Unknown server 'nosuch'. Choose one of: autovisualiser, computercontroller, \
             webdocuments, developer, memory."
        );
    }

    /// Every server parses back from the name it is listed by.
    #[test]
    fn every_listed_name_parses_back_to_its_server() {
        for server in McpCommand::ALL {
            assert_eq!(McpCommand::from_str(server.name()), Ok(server));
        }
        assert_eq!(
            McpCommand::from_str("Web Documents"),
            Ok(McpCommand::WebDocuments),
            "case and spaces are still ignored"
        );
    }

    #[test]
    fn retired_tutorial_is_not_registered_or_runnable() {
        assert!(McpCommand::from_str("tutorial").is_err());
        assert!(!crate::BUILTIN_EXTENSIONS.contains_key("tutorial"));
    }
}
