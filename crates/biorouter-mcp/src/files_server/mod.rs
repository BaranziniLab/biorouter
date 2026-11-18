//! Jailed file-access MCP server for BRSDK apps — the "access files" capability.
//!
//! A purpose-built, **jailed-by-construction** server: every path goes through
//! the workspace [`Jail`](crate::developer::jail::Jail) (canonicalize + prefix +
//! symlink-escape defence + `.vault`/`.git` denial + read-only enforcement), so
//! an app's file tools are confined to its own workspace. Constructed per app
//! and injected via `ExtensionManager::add_inprocess_server` (the same seam the
//! datasql server uses).
//!
//! An app's manifest may name **entries** (`capabilities.files.entries`), each
//! a folder `workspace/<name>`. With entries, the tools still read anywhere in
//! the workspace, and write only into an entry that is `"rw"` or an `out_dir`;
//! a `"ro"` entry, and everything outside the entries, refuses every write.
//! Without entries the whole workspace is writable, as it always was.

use std::path::{Component, Path, PathBuf};

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, Content, ErrorCode, ErrorData, Implementation, ServerCapabilities,
        ServerInfo,
    },
    tool, tool_handler, tool_router, ServerHandler,
};
use serde::{Deserialize, Serialize};

use crate::agent_drafter::manifest::FilesCapability;
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

/// One manifest entry as the server enforces it.
#[derive(Debug, Clone)]
struct EntryRule {
    name: String,
    writable: bool,
}

/// Jailed file server for one app's workspace.
#[derive(Clone)]
pub struct FilesServer {
    tool_router: ToolRouter<Self>,
    jail: Jail,
    /// The manifest's entries. `None`: no entries were declared, and the
    /// jail's own `writable` flag decides every write.
    entries: Option<Vec<EntryRule>>,
    /// Cap on one read or write, from `max_file_bytes` (at most the read cap).
    max_bytes: u64,
}

impl FilesServer {
    pub fn new(jail: Jail) -> Self {
        Self {
            tool_router: Self::tool_router(),
            jail,
            entries: None,
            max_bytes: MAX_READ_BYTES,
        }
    }

    /// Why a write to `relative` (the path as the jail resolved it, links
    /// followed) is refused under the manifest's entries, or `None` when the
    /// write may proceed.
    fn entry_refusal(&self, requested: &str, relative: &Path) -> Option<String> {
        let entries = self.entries.as_ref()?;
        let first = relative.components().find_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().to_string()),
            _ => None,
        });
        let writable: Vec<&str> = entries
            .iter()
            .filter(|entry| entry.writable)
            .map(|entry| entry.name.as_str())
            .collect();
        let where_to = if writable.is_empty() {
            "This app has no writable file entry.".to_string()
        } else {
            format!("It can write only into: {}.", writable.join(", "))
        };
        let Some(first) = first else {
            return Some(format!(
                "'{requested}' is not inside a writable entry. {where_to}"
            ));
        };
        // A read-only entry wins over a writable one of the same name, and is
        // matched without regard to case: on a case-insensitive disk `DATA/x`
        // is the same folder as `data/x`.
        if entries
            .iter()
            .any(|entry| !entry.writable && entry.name.eq_ignore_ascii_case(&first))
        {
            return Some(format!(
                "'{requested}' is in the read-only entry '{first}'. {where_to}"
            ));
        }
        if entries
            .iter()
            .any(|entry| entry.writable && entry.name == first)
        {
            return None;
        }
        Some(format!(
            "'{requested}' is not inside a writable entry. {where_to}"
        ))
    }

    /// What the server tells the model about where it may write.
    fn instructions(&self) -> String {
        let base = "Read, write, and list files inside this app's workspace (and only there). \
                    Paths are workspace-relative.";
        let Some(entries) = self.entries.as_ref() else {
            return base.to_string();
        };
        let described: Vec<String> = entries
            .iter()
            .map(|entry| {
                format!(
                    "{} ({})",
                    entry.name,
                    if entry.writable {
                        "writable"
                    } else {
                        "read-only"
                    }
                )
            })
            .collect();
        format!(
            "{base} Writes go only into a writable entry; everything else is read-only. \
             Entries: {}.",
            described.join(", ")
        )
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
            instructions: Some(self.instructions()),
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
        if meta.len() > self.max_bytes {
            return Err(invalid(format!(
                "file '{p}' is {} bytes, exceeds the {}-byte read cap",
                meta.len(),
                self.max_bytes
            )));
        }
        let content = path
            .read_to_string_limited(self.max_bytes)
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
        if let Some(refusal) = self.entry_refusal(&p, path.relative_path()) {
            return Err(invalid(refusal));
        }
        if content.len() as u64 > self.max_bytes {
            return Err(invalid(format!(
                "'{p}' would be {} bytes, over this app's {}-byte file cap",
                content.len(),
                self.max_bytes
            )));
        }
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

/// Build the [`FilesServer`] an app's `files` capability grants.
///
/// Each entry is the folder `workspace/<name>`, created here. An entry whose
/// name is not one plain folder name (empty, `.`, `..`, a path, or a denied
/// name such as `.vault`) is skipped, and a manifest whose entries are all
/// skipped can write nowhere: it asked for entries. An entry's `local_dir` is
/// never mounted (owner decision, rq2-199); the app's socket reports it as
/// withheld. With no entries the whole workspace is writable, as it always was.
pub fn for_app(workspace: PathBuf, files: &FilesCapability) -> FilesServer {
    let mut rules: Vec<EntryRule> = Vec::new();
    for entry in &files.entries {
        let name = entry.name.trim();
        let plain = !name.is_empty()
            && Path::new(name).components().count() == 1
            && matches!(
                Path::new(name).components().next(),
                Some(Component::Normal(_))
            )
            && !matches!(name, ".vault" | ".git");
        if !plain {
            tracing::warn!(entry = %entry.name, "files entry name is not a plain folder name; skipped");
            continue;
        }
        let _ = std::fs::create_dir_all(workspace.join(name));
        rules.push(EntryRule {
            name: name.to_string(),
            writable: entry.is_writable(),
        });
    }
    let mut server = FilesServer::new(Jail::new(workspace, true));
    if !files.entries.is_empty() {
        server.entries = Some(rules);
    }
    if let Some(cap) = files.max_file_bytes.filter(|cap| *cap > 0) {
        server.max_bytes = cap.min(MAX_READ_BYTES);
    }
    server
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

    fn capability(entries: serde_json::Value) -> FilesCapability {
        serde_json::from_value(serde_json::json!({ "entries": entries })).unwrap()
    }

    async fn write(srv: &FilesServer, path: &str) -> Result<CallToolResult, ErrorData> {
        srv.files_write(Parameters(WriteParams {
            path: path.to_string(),
            content: "written".to_string(),
        }))
        .await
    }

    /// rq2-199: a manifest entry with `mode: "ro"` is read only. The files
    /// server used to be built writable everywhere and never read the entries,
    /// so a write into a read-only entry succeeded.
    #[tokio::test]
    async fn a_read_only_entry_refuses_writes_and_a_rw_entry_accepts_them() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("workspace");
        std::fs::create_dir_all(ws.join("data")).unwrap();
        std::fs::write(ws.join("data/note.txt"), "hello").unwrap();
        let srv = for_app(
            ws.clone(),
            &capability(serde_json::json!([
                { "name": "data", "mode": "ro" },
                { "name": "out", "mode": "rw" },
                { "name": "results", "out_dir": true }
            ])),
        );

        let read = srv
            .files_read(Parameters(ReadParams {
                path: "data/note.txt".to_string(),
            }))
            .await
            .expect("a read-only entry is readable");
        assert!(text(&read).contains("hello"));

        // A link inside a writable entry that leads into the read-only one is
        // judged where it leads.
        let mut refused = vec!["data/note.txt", "data/new.txt", "loose.txt"];
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("../data", ws.join("out/link")).unwrap();
            refused.push("out/link/sneaky.txt");
        }
        for refused in refused {
            let error = write(&srv, refused).await.expect_err(refused);
            assert!(
                error.message.contains("read-only") || error.message.contains("entr"),
                "{refused}: the refusal says why: {}",
                error.message
            );
        }
        // A `..` step is refused by the jail before any entry is asked.
        write(&srv, "out/../data/sneaky.txt")
            .await
            .expect_err("a traversal into the read-only entry");
        assert_eq!(
            std::fs::read_to_string(ws.join("data/note.txt")).unwrap(),
            "hello",
            "the read-only file is unchanged"
        );
        assert!(!ws.join("data/new.txt").exists());
        assert!(!ws.join("data/sneaky.txt").exists());
        assert!(!ws.join("loose.txt").exists());

        write(&srv, "out/report.txt")
            .await
            .expect("an rw entry accepts a write");
        write(&srv, "results/table.csv")
            .await
            .expect("an out_dir entry accepts a write");
        assert!(ws.join("out/report.txt").is_file());
        assert!(ws.join("results/table.csv").is_file());
    }

    /// A `files` grant with no entries keeps the whole workspace writable, as
    /// it always was.
    #[tokio::test]
    async fn a_files_grant_without_entries_keeps_the_workspace_writable() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("workspace");
        std::fs::create_dir_all(&ws).unwrap();
        let srv = for_app(ws.clone(), &FilesCapability::default());
        write(&srv, "notes/a.txt")
            .await
            .expect("no entries: the workspace is writable");
        assert!(ws.join("notes/a.txt").is_file());
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
