//! Jailed file-access MCP server for BRSDK apps — the "access files" capability.
//!
//! A purpose-built, **jailed-by-construction** server: every path goes through
//! the workspace [`Jail`](crate::developer::jail::Jail) (canonicalize + prefix +
//! symlink-escape defence + `.vault`/`.git` denial + read-only enforcement), so
//! an app's file tools are confined to its own workspace. Constructed per app
//! and injected via `ExtensionManager::add_inprocess_server` (the same seam the
//! datasql server uses).

use std::path::PathBuf;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, Content, ErrorCode, ErrorData, Implementation, ServerCapabilities,
        ServerInfo,
    },
    tool, tool_handler, tool_router, ServerHandler,
};
use serde::{Deserialize, Serialize};

use crate::developer::jail::Jail;

const MAX_READ_BYTES: u64 = 5 * 1024 * 1024; // 5 MiB read cap

fn invalid(msg: impl Into<String>) -> ErrorData {
    ErrorData::new(ErrorCode::INVALID_PARAMS, msg.into(), None)
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct ListParams {
    /// Workspace-relative directory (default: the workspace root).
    #[serde(default)]
    pub dir: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct ReadParams {
    /// Workspace-relative file path.
    pub path: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
pub struct WriteParams {
    /// Workspace-relative file path.
    pub path: String,
    /// UTF-8 content to write.
    pub content: String,
}

/// Jailed file server for one app's workspace.
#[derive(Clone)]
pub struct FilesServer {
    tool_router: ToolRouter<Self>,
    jail: Jail,
}

impl FilesServer {
    pub fn new(jail: Jail) -> Self {
        Self {
            tool_router: Self::tool_router(),
            jail,
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for FilesServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: "biorouter-files".to_string(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                title: None,
                icons: None,
                website_url: None,
            },
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(
                "Read, write, and list files inside this app's workspace (and only there). \
                 Paths are workspace-relative."
                    .to_string(),
            ),
            ..Default::default()
        }
    }
}

#[tool_router(router = tool_router)]
impl FilesServer {
    #[tool(
        name = "files_list",
        description = "List entries of a workspace directory (default: the workspace root)."
    )]
    pub async fn files_list(
        &self,
        params: Parameters<ListParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let dir = params.0.dir.unwrap_or_else(|| ".".to_string());
        let path = self
            .jail
            .bind(&dir, false)
            .map_err(|e| invalid(e.to_string()))?;
        let mut entries: Vec<serde_json::Value> = Vec::new();
        for entry in path
            .list()
            .map_err(|error| invalid(format!("cannot list '{dir}': {error}")))?
        {
            entries.push(serde_json::json!({
                "name": entry.name.to_string_lossy(),
                "dir": entry.is_dir,
                "size": entry.len,
            }));
        }
        let json = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        name = "files_read",
        description = "Read a UTF-8 text file from the workspace."
    )]
    pub async fn files_read(
        &self,
        params: Parameters<ReadParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = params.0.path;
        let path = self
            .jail
            .bind(&p, false)
            .map_err(|e| invalid(e.to_string()))?;
        let meta = path
            .metadata()
            .map_err(|e| invalid(format!("cannot stat '{p}': {e}")))?;
        if meta.len() > MAX_READ_BYTES {
            return Err(invalid(format!(
                "file '{p}' is {} bytes, exceeds the {MAX_READ_BYTES}-byte read cap",
                meta.len()
            )));
        }
        let content = path
            .read_to_string_limited(MAX_READ_BYTES)
            .map_err(|e| invalid(format!("cannot read '{p}': {e}")))?;
        Ok(CallToolResult::success(vec![Content::text(content)]))
    }

    #[tool(
        name = "files_write",
        description = "Write a UTF-8 text file into the workspace (creates parent dirs). Requires a read-write workspace."
    )]
    pub async fn files_write(
        &self,
        params: Parameters<WriteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let WriteParams { path: p, content } = params.0;
        // need_write=true → the jail rejects this if the workspace is read-only.
        let path = self
            .jail
            .bind(&p, true)
            .map_err(|e| invalid(e.to_string()))?;
        path.atomic_write(content.as_bytes())
            .map_err(|error| invalid(format!("cannot write '{p}': {error}")))?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "wrote {p}"
        ))]))
    }
}

/// Build a [`FilesServer`] for an app workspace. `writable` gates all writes.
pub fn for_workspace(workspace: PathBuf, writable: bool) -> FilesServer {
    FilesServer::new(Jail::new(workspace, writable))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(writable: bool) -> (tempfile::TempDir, FilesServer) {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("workspace");
        std::fs::create_dir_all(&ws).unwrap();
        (dir, for_workspace(ws, writable))
    }

    fn text(r: &CallToolResult) -> String {
        serde_json::to_string(r).unwrap()
    }

    #[tokio::test]
    async fn write_then_read_roundtrip() {
        let (_d, srv) = server(true);
        srv.files_write(Parameters(WriteParams {
            path: "notes/result.txt".to_string(),
            content: "CFTR analysis done".to_string(),
        }))
        .await
        .unwrap();
        let r = srv
            .files_read(Parameters(ReadParams {
                path: "notes/result.txt".to_string(),
            }))
            .await
            .unwrap();
        assert!(text(&r).contains("CFTR analysis done"));
    }

    #[tokio::test]
    async fn read_rejects_traversal() {
        let (_d, srv) = server(true);
        let r = srv
            .files_read(Parameters(ReadParams {
                path: "../../etc/passwd".to_string(),
            }))
            .await;
        assert!(r.is_err(), "traversal read must be rejected by the jail");
    }

    #[tokio::test]
    async fn write_rejected_in_readonly_workspace() {
        let (_d, srv) = server(false); // read-only
        let r = srv
            .files_write(Parameters(WriteParams {
                path: "x.txt".to_string(),
                content: "nope".to_string(),
            }))
            .await;
        assert!(
            r.is_err(),
            "write to a read-only workspace must be rejected"
        );
    }

    #[tokio::test]
    async fn list_returns_entries_and_denies_vault() {
        let (_d, srv) = server(true);
        srv.files_write(Parameters(WriteParams {
            path: "a.txt".to_string(),
            content: "1".to_string(),
        }))
        .await
        .unwrap();
        let r = srv
            .files_list(Parameters(ListParams { dir: None }))
            .await
            .unwrap();
        assert!(text(&r).contains("a.txt"));
        // The vault dir is denied even for listing.
        let v = srv
            .files_list(Parameters(ListParams {
                dir: Some(".vault".to_string()),
            }))
            .await;
        assert!(v.is_err(), ".vault must be denied by the jail");
    }
}
