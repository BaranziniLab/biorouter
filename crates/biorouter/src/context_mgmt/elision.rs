//! Stale tool-result elision: older, larger tool results leave the MODEL's
//! copy of the conversation, in rare persisted batches, and are replaced by a
//! short stub that says what the result was and where its full text is saved.
//!
//! Why. Every request re-sends the whole conversation. In the long data
//! analysis chat that motivated this (575 requests, a median prompt of about
//! 270k tokens), tool results were 58% of everything re-read: a 20k-character
//! query result from request 40 was sent again with each of the 500 requests
//! after it. The per-result cap (`large_response_handler`, 10k tokens) does not
//! catch those, because each one is under it.
//!
//! What stays the same:
//!   * The person's transcript. Only the per-request clone the provider is
//!     sent is changed ([`apply_elisions`] runs inside
//!     `Agent::assemble_turn_context`); the stored messages, the desktop, the
//!     CLI, exports and compaction's summarizer all keep the full text.
//!   * Tool-call pairing. A stub keeps the tool response's id and its place
//!     right after its request, so every provider format stays valid. The
//!     tool call's ARGUMENTS stay too: the model still sees what it asked for.
//!   * The data. The full text is written to the same
//!     `<working dir>/.biorouter/tool-output/` folder the large-response
//!     handler uses (or the file that handler already wrote is named), and the
//!     stub says to read or search that file instead of re-running the tool.
//!     When the file cannot be written, that result is not elided.
//!
//! What is protected. The newest [`DEFAULT_PROTECT_TOKENS`] of tool output is
//! kept verbatim, and so is every result of the latest round (the responses to
//! the newest assistant message), whatever their size. Errors keep their first
//! lines in the stub; results of the checklist, skills and workspace protocol
//! tools ([`is_exempt_tool`]) and results with images are never elided.
//!
//! Prompt cache. A provider caches the request's PREFIX, and any change throws
//! the cache away from that byte on. So the set of elided results is persisted
//! in the chat (`extension_data`, key `tool_elision.v1`) and grows only in
//! batches: nothing is elided until at least [`DEFAULT_MIN_PRUNABLE_TOKENS`] of
//! older output is eligible, and then all of it goes at once. Between batches
//! every request is the previous one plus the new tail, byte for byte, because
//! a stub is a pure function of its stored entry ([`render_stub`]). A sliding
//! rewrite every turn would cost more than it saves: simulated on the
//! motivating chat, it broke the cache on 278 requests and made cache-priced
//! spend 7.6% HIGHER than doing nothing.
//!
//! Configuration (`config.yaml` or environment):
//!   * `BIOROUTER_TOOL_RESULT_ELISION` (default `true`): the kill switch.
//!     `false` sends every result in full again.
//!   * `BIOROUTER_TOOL_RESULT_ELISION_PROTECT_TOKENS` (default 40,000).
//!   * `BIOROUTER_TOOL_RESULT_ELISION_MIN_PRUNABLE_TOKENS` (default 25,000).
//!   * `BIOROUTER_TOOL_RESULT_ELISION_MIN_CHARS` (default 1,500).

use crate::config::Config;
use crate::conversation::message::{Message, MessageContent, ToolResponse};
use crate::conversation::Conversation;
use crate::session::extension_data::ExtensionState;
use rmcp::model::{CallToolResult, Content, Role};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Kill switch. `false` sends every tool result in full, including results an
/// earlier batch elided.
pub const ELISION_ENABLED_KEY: &str = "BIOROUTER_TOOL_RESULT_ELISION";
/// Newest tool output, in estimated tokens, that is never elided.
pub const PROTECT_TOKENS_KEY: &str = "BIOROUTER_TOOL_RESULT_ELISION_PROTECT_TOKENS";
/// How much older output must be eligible before a batch runs.
pub const MIN_PRUNABLE_TOKENS_KEY: &str = "BIOROUTER_TOOL_RESULT_ELISION_MIN_PRUNABLE_TOKENS";
/// A result shorter than this many characters is never elided.
pub const MIN_CHARS_KEY: &str = "BIOROUTER_TOOL_RESULT_ELISION_MIN_CHARS";

/// The newest tool output kept verbatim. The same figure opencode uses
/// (`PRUNE_PROTECT`); Gemini CLI keeps 50k.
pub const DEFAULT_PROTECT_TOKENS: usize = 40_000;
/// A batch needs at least this much eligible older output, so the cache is
/// broken rarely and only for a real saving (opencode 20k, Gemini CLI 30k).
pub const DEFAULT_MIN_PRUNABLE_TOKENS: usize = 25_000;
/// Results under this size cost less than the cache break of eliding them.
pub const DEFAULT_MIN_CHARS: usize = 1_500;

/// The estimate of tokens per character used for every threshold here. Cheap
/// and deterministic (the same input always plans the same batch); it errs low
/// on JSON and code (about 3.3 characters a token), so a batch runs a little
/// later than the figures say rather than earlier.
const CHARS_PER_TOKEN: usize = 4;

/// How much of an error's text a stub keeps.
const ERROR_HEAD_LINES: usize = 3;
const ERROR_HEAD_CHARS: usize = 400;
/// How much of a successful result's first line a stub keeps, as a reminder
/// of what it was.
const FIRST_LINE_CHARS: usize = 120;

/// Same folder as `large_response_handler`'s handles, so the model finds every
/// saved result in one place.
const HANDLE_DIR: &str = ".biorouter/tool-output";

/// The large-response handler's fallback when the working dir cannot be
/// written.
fn fallback_handle_dir() -> PathBuf {
    std::env::temp_dir().join("biorouter_mcp_responses")
}
/// The line `large_response_handler` writes when it already saved a result.
const OFFLOADED_MARKER: &str = "The complete output is saved at: ";

/// The thresholds, resolved from config once per request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElisionParams {
    pub enabled: bool,
    pub protect_tokens: usize,
    pub min_prunable_tokens: usize,
    pub min_chars: usize,
}

impl Default for ElisionParams {
    fn default() -> Self {
        Self {
            enabled: true,
            protect_tokens: DEFAULT_PROTECT_TOKENS,
            min_prunable_tokens: DEFAULT_MIN_PRUNABLE_TOKENS,
            min_chars: DEFAULT_MIN_CHARS,
        }
    }
}

impl ElisionParams {
    pub fn from_config() -> Self {
        let config = Config::global();
        let defaults = Self::default();
        Self {
            enabled: config
                .get_param::<bool>(ELISION_ENABLED_KEY)
                .unwrap_or(defaults.enabled),
            protect_tokens: config
                .get_param::<usize>(PROTECT_TOKENS_KEY)
                .unwrap_or(defaults.protect_tokens),
            min_prunable_tokens: config
                .get_param::<usize>(MIN_PRUNABLE_TOKENS_KEY)
                .unwrap_or(defaults.min_prunable_tokens),
            min_chars: config
                .get_param::<usize>(MIN_CHARS_KEY)
                .unwrap_or(defaults.min_chars),
        }
    }
}

/// One elided result, as persisted. Everything its stub says is stored here,
/// so the stub never depends on re-reading anything and is byte-identical on
/// every request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElidedResult {
    /// The tool that produced it, as the model called it.
    pub tool: String,
    /// Where its full text is saved.
    pub path: String,
    pub chars: usize,
    pub lines: usize,
    /// For an error, its first lines; otherwise the start of its first line.
    pub head: String,
    #[serde(default)]
    pub is_error: bool,
}

/// The chat's elided results, keyed by tool call id. A `BTreeMap` so the
/// stored JSON is stable too.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResultElisions {
    #[serde(default)]
    pub entries: BTreeMap<String, ElidedResult>,
}

impl ExtensionState for ToolResultElisions {
    const EXTENSION_NAME: &'static str = "tool_elision";
    const VERSION: &'static str = "v1";
}

/// Tools whose results are never elided: the checklist (the model's own plan),
/// skills (instructions it was told to follow), and the workspace protocol
/// receipts other code reads back. Both name forms.
pub fn is_exempt_tool(name: &str) -> bool {
    let bare = name.rsplit("__").next().unwrap_or(name);
    let extension = name.split_once("__").map(|(ext, _)| ext);
    matches!(extension, Some("todo") | Some("skills"))
        || bare.starts_with("todo_")
        || matches!(
            bare,
            "loadSkill" | "workspace_watch" | "workspace_read_conversation" | "read_session_blob"
        )
}

/// A result the next batch elides: what to save, and what its stub will say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElisionCandidate {
    pub id: String,
    pub tool: String,
    /// The model-visible text of the result, joined.
    pub text: String,
    /// The path the result's text says `large_response_handler` saved it to.
    /// A CLAIM read from tool output, which anything the tool read can forge:
    /// `save_candidates` reuses it only when it is that handler's own file for
    /// this chat, and otherwise writes a fresh one.
    pub existing_handle: Option<String>,
    pub is_error: bool,
    pub estimated_tokens: usize,
}

fn estimate_tokens(chars: usize) -> usize {
    chars.div_ceil(CHARS_PER_TOKEN)
}

/// The text of a successful result the model would be sent, or `None` when the
/// result is not something a stub can stand in for (an `Err`, or a result
/// carrying an image or other non-text content for the model).
fn model_text(result: &CallToolResult) -> Option<String> {
    let mut texts = Vec::new();
    for content in &result.content {
        if !crate::providers::formats::audience::is_for_model(content) {
            continue;
        }
        match content.as_text() {
            Some(text) => texts.push(text.text.as_str()),
            None => return None,
        }
    }
    Some(texts.join("\n"))
}

fn tool_names_by_id(messages: &[&Message]) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for message in messages {
        for content in &message.content {
            if let MessageContent::ToolRequest(request) = content {
                if let Ok(call) = &request.tool_call {
                    names.insert(request.id.clone(), call.name.to_string());
                }
            }
        }
    }
    names
}

/// Decide the next batch: the results to elide now, or nothing.
///
/// Pure and deterministic. Walks the agent-visible tool results newest first,
/// keeping verbatim everything up to `protect_tokens` (an already elided result
/// counts as its stub), and every result of the latest round. Older results
/// over `min_chars` that are not exempt, not errors without text and not
/// already elided are eligible; when they add up to `min_prunable_tokens` they
/// are ALL returned, and otherwise none is.
pub fn plan_elision(
    messages: &[Message],
    state: &ToolResultElisions,
    params: &ElisionParams,
) -> Vec<ElisionCandidate> {
    if !params.enabled {
        return Vec::new();
    }
    let visible: Vec<&Message> = messages.iter().filter(|m| m.is_agent_visible()).collect();
    let names = tool_names_by_id(&visible);
    // The latest round: the responses that follow the newest assistant
    // message. The model asked for them on its last request and has not seen
    // them yet.
    let latest_round_starts = visible
        .iter()
        .rposition(|m| m.role == Role::Assistant)
        .map_or(0, |i| i + 1);

    let mut seen_tokens = 0usize;
    let mut prunable_tokens = 0usize;
    let mut candidates = Vec::new();
    for (index, message) in visible.iter().enumerate().rev() {
        for content in message.content.iter().rev() {
            let MessageContent::ToolResponse(response) = content else {
                continue;
            };
            if let Some(entry) = state.entries.get(&response.id) {
                seen_tokens += estimate_tokens(render_stub(entry).chars().count());
                continue;
            }
            let Ok(result) = &response.tool_result else {
                continue;
            };
            let text = model_text(result);
            let chars = text.as_deref().map_or(0, |t| t.chars().count());
            seen_tokens += estimate_tokens(chars);
            if index >= latest_round_starts || seen_tokens <= params.protect_tokens {
                continue;
            }
            let Some(text) = text else { continue };
            if chars < params.min_chars {
                continue;
            }
            let tool = names
                .get(&response.id)
                .cloned()
                .unwrap_or_else(|| "tool".to_string());
            if is_exempt_tool(&tool) {
                continue;
            }
            let estimated_tokens = estimate_tokens(chars);
            prunable_tokens += estimated_tokens;
            candidates.push(ElisionCandidate {
                id: response.id.clone(),
                existing_handle: existing_handle(&text),
                is_error: result.is_error == Some(true),
                tool,
                text,
                estimated_tokens,
            });
        }
    }
    if prunable_tokens < params.min_prunable_tokens {
        return Vec::new();
    }
    // Oldest first, which is also the order the files are written in.
    candidates.reverse();
    candidates
}

/// The path the result's text names as `large_response_handler`'s saved copy.
/// Only a claim: see [`trusted_existing_handle`].
fn existing_handle(text: &str) -> Option<String> {
    let path = text
        .lines()
        .find_map(|l| l.strip_prefix(OFFLOADED_MARKER))?
        .trim();
    (!path.is_empty()).then(|| path.to_string())
}

/// `claimed` when it is a file `large_response_handler` wrote for this chat:
/// a regular file whose real location is directly inside the chat's
/// `.biorouter/tool-output` folder or the handler's fallback folder, named the
/// way the handler names this session's handles. The marker line sits in the
/// tool's own output, so a scraped page, a document or a query row can print
/// it with any path; the stub is host-authored text outside the untrusted
/// frame, and must never send the model to a file the tool did not write.
fn trusted_existing_handle(
    claimed: &str,
    session_id: &str,
    working_dir: &Path,
    fallback_dir: &Path,
) -> Option<String> {
    let real = std::fs::canonicalize(claimed).ok()?;
    if !std::fs::metadata(&real).ok()?.is_file() {
        return None;
    }
    let name = real.file_name()?.to_str()?;
    if !crate::agents::large_response_handler::is_handle_filename_for_session(name, session_id) {
        return None;
    }
    let parent = real.parent()?;
    let inside = [working_dir.join(HANDLE_DIR), fallback_dir.to_path_buf()]
        .iter()
        .filter_map(|dir| std::fs::canonicalize(dir).ok())
        .any(|dir| dir == parent);
    inside.then(|| claimed.to_string())
}

fn head_of(text: &str, is_error: bool) -> String {
    if is_error {
        let head: String = text
            .lines()
            .take(ERROR_HEAD_LINES)
            .collect::<Vec<_>>()
            .join("\n");
        return truncate_chars(&head, ERROR_HEAD_CHARS);
    }
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    truncate_chars(first.trim(), FIRST_LINE_CHARS)
}

fn truncate_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", text.split_at(cut).0),
        None => text.to_string(),
    }
}

/// What the model reads in place of an elided result. A pure function of the
/// stored entry, so it is byte-identical on every request.
pub fn render_stub(entry: &ElidedResult) -> String {
    let mut out = format!(
        "[Earlier tool result moved out of the conversation to save context.] `{tool}` returned \
         {chars} characters ({lines} lines) here{error}. The full result is saved at: {path}\n\
         If you need it again, read or search that file with your file or shell tools (a pattern \
         or a line range). Do not re-run the tool just to see it.",
        tool = entry.tool,
        chars = entry.chars,
        lines = entry.lines,
        error = if entry.is_error { ", as an error" } else { "" },
        path = entry.path,
    );
    if !entry.head.is_empty() {
        if entry.is_error {
            out.push_str("\nIts first lines:\n");
        } else {
            out.push_str("\nIt began: ");
        }
        out.push_str(&entry.head);
    }
    out
}

/// Replace every elided result in `conversation` with its stub. Only the
/// model-visible text changes: the response keeps its id, its place, its
/// `is_error` flag, its metadata and any content meant for the person alone.
pub fn apply_elisions(conversation: Conversation, state: &ToolResultElisions) -> Conversation {
    if state.entries.is_empty() {
        return conversation;
    }
    let mut messages = conversation.into_messages();
    for message in messages.iter_mut().filter(|m| m.is_agent_visible()) {
        for content in message.content.iter_mut() {
            let MessageContent::ToolResponse(response) = content else {
                continue;
            };
            let Some(entry) = state.entries.get(&response.id) else {
                continue;
            };
            stub_response(response, entry);
        }
    }
    Conversation::new_unvalidated(messages)
}

fn stub_response(response: &mut ToolResponse, entry: &ElidedResult) {
    let Ok(result) = &mut response.tool_result else {
        return;
    };
    let stub = Content::text(render_stub(entry));
    let mut replaced = false;
    let mut kept = Vec::with_capacity(result.content.len());
    for content in std::mem::take(&mut result.content) {
        if !crate::providers::formats::audience::is_for_model(&content) {
            kept.push(content);
        } else if !replaced {
            kept.push(stub.clone());
            replaced = true;
        }
    }
    if !replaced {
        kept.push(stub);
    }
    result.content = kept;
    result.structured_content = None;
}

/// Save each candidate's full text and build its stored entry. A candidate
/// whose text cannot be saved is left out, so it is not elided. Blocking file
/// I/O; call it on the blocking pool.
fn save_candidates(
    candidates: Vec<ElisionCandidate>,
    session_id: &str,
    working_dir: &Path,
    fallback_dir: &Path,
) -> Vec<(String, ElidedResult)> {
    let mut saved = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let reused = candidate.existing_handle.as_deref().and_then(|claimed| {
            trusted_existing_handle(claimed, session_id, working_dir, fallback_dir)
        });
        let path = match reused {
            Some(path) => path,
            None => match write_handle(&candidate, session_id, working_dir, fallback_dir) {
                Ok(path) => path.display().to_string(),
                Err(e) => {
                    warn!(
                        tool = %candidate.tool,
                        "could not save an older tool result before eliding it ({e}); \
                         keeping it in the conversation"
                    );
                    continue;
                }
            },
        };
        saved.push((
            candidate.id.clone(),
            ElidedResult {
                head: head_of(&candidate.text, candidate.is_error),
                chars: candidate.text.chars().count(),
                lines: candidate.text.lines().count(),
                is_error: candidate.is_error,
                tool: candidate.tool,
                path,
            },
        ));
    }
    saved
}

fn write_handle(
    candidate: &ElisionCandidate,
    session_id: &str,
    working_dir: &Path,
    fallback_dir: &Path,
) -> std::io::Result<PathBuf> {
    let filename = format!(
        "elided-{}-{}-{}.txt",
        sanitize(&candidate.tool, "tool"),
        sanitize(session_id, "session"),
        sanitize(&candidate.id, "call"),
    );
    let in_workspace = working_dir.join(HANDLE_DIR);
    if prepare_handle_dir(&in_workspace).is_ok() {
        let path = in_workspace.join(&filename);
        if std::fs::write(&path, candidate.text.as_bytes()).is_ok() {
            return Ok(path);
        }
    }
    // The large-response handler's own fallback: an awkward place beats losing
    // the text.
    std::fs::create_dir_all(fallback_dir)?;
    let path = fallback_dir.join(&filename);
    std::fs::write(&path, candidate.text.as_bytes())?;
    Ok(path)
}

/// Create the folder and keep it out of git, as `large_response_handler` does.
fn prepare_handle_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    if let Some(parent) = dir.parent() {
        let gitignore = parent.join(".gitignore");
        if !gitignore.exists() {
            let _ = std::fs::write(&gitignore, "*\n");
        }
    }
    Ok(())
}

fn sanitize(raw: &str, fallback: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    if cleaned.trim_matches('_').is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

/// The ids of every tool response still in `messages`, so a stored entry for a
/// message that is gone (a rewind, an edit) can be dropped.
fn response_ids(messages: &[Message]) -> HashSet<String> {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            MessageContent::ToolResponse(r) => Some(r.id.clone()),
            _ => None,
        })
        .collect()
}

/// The whole step, run before every model request of a turn: load the chat's
/// elisions, run a batch when one is due (save the texts, persist the grown
/// set), and return the conversation with every elided result stubbed.
///
/// Never fails the request. A state that cannot be read applies nothing new
/// and plans nothing (planning against a state it could not read would elide
/// the same results twice under new names); a batch that cannot be persisted
/// is not applied, so the next request plans it again rather than sending
/// stubs the following one would not.
///
/// `can_read_saved_output` says whether the chat has a tool that can read the
/// saved file back (Developer's shell or text editor, directly or from a Code
/// Execution script). Without one, no new batch is planned, since a stub would
/// point at text the model could not reach; results elided earlier stay elided,
/// so the prefix does not move.
pub async fn elide_stale_tool_results(
    session_manager: &crate::session::SessionManager,
    session_id: &str,
    conversation: Conversation,
    working_dir: &Path,
    can_read_saved_output: bool,
) -> Conversation {
    let params = ElisionParams::from_config();
    if !params.enabled {
        return conversation;
    }
    let state = match session_manager
        .get_extension_state(
            session_id,
            ToolResultElisions::EXTENSION_NAME,
            ToolResultElisions::VERSION,
        )
        .await
    {
        Ok(Some(value)) => match ToolResultElisions::from_value(&value) {
            Ok(state) => state,
            Err(e) => {
                debug!("tool-result elision state unreadable ({e}); sending results in full");
                return conversation;
            }
        },
        Ok(None) => ToolResultElisions::default(),
        Err(e) => {
            debug!("could not read tool-result elision state ({e}); sending results in full");
            return conversation;
        }
    };

    let candidates = if can_read_saved_output {
        plan_elision(conversation.messages(), &state, &params)
    } else {
        Vec::new()
    };
    let state = if candidates.is_empty() {
        state
    } else {
        match run_batch(
            session_manager,
            session_id,
            &conversation,
            working_dir,
            candidates,
        )
        .await
        {
            Some(grown) => grown,
            None => state,
        }
    };
    apply_elisions(conversation, &state)
}

/// The conversation as the model is sent it, using the elisions stored on
/// `session` (`session.extension_data`); `None` when none apply, so a caller
/// keeps using its own copy without cloning it. For measuring a request, as
/// the compaction check does: it plans nothing and writes nothing.
pub fn agent_view_for_session(
    conversation: &Conversation,
    session: &crate::session::Session,
) -> Option<Conversation> {
    if !ElisionParams::from_config().enabled {
        return None;
    }
    let state = ToolResultElisions::from_extension_data(&session.extension_data)?;
    if state.entries.is_empty() {
        return None;
    }
    Some(apply_elisions(conversation.clone(), &state))
}

async fn run_batch(
    session_manager: &crate::session::SessionManager,
    session_id: &str,
    conversation: &Conversation,
    working_dir: &Path,
    candidates: Vec<ElisionCandidate>,
) -> Option<ToolResultElisions> {
    let planned = candidates.len();
    let planned_tokens: usize = candidates.iter().map(|c| c.estimated_tokens).sum();
    let owned_session = session_id.to_string();
    let owned_dir = working_dir.to_path_buf();
    let saved = match tokio::task::spawn_blocking(move || {
        save_candidates(
            candidates,
            &owned_session,
            &owned_dir,
            &fallback_handle_dir(),
        )
    })
    .await
    {
        Ok(saved) => saved,
        Err(e) => {
            warn!("tool-result elision: saving task failed ({e})");
            return None;
        }
    };
    if saved.is_empty() {
        return None;
    }
    let live_ids = response_ids(conversation.messages());
    let written = session_manager
        .update_extension_state(
            session_id,
            ToolResultElisions::EXTENSION_NAME,
            ToolResultElisions::VERSION,
            move |current| {
                let mut state = match current {
                    Some(value) => ToolResultElisions::from_value(value)?,
                    None => ToolResultElisions::default(),
                };
                state.entries.retain(|id, _| live_ids.contains(id));
                state.entries.extend(saved);
                state.to_value()
            },
        )
        .await;
    match written {
        Ok(Some(value)) => match ToolResultElisions::from_value(&value) {
            Ok(state) => {
                info!(
                    session_id,
                    elided = planned,
                    estimated_tokens = planned_tokens,
                    total_elided = state.entries.len(),
                    "elided older tool results from the model's context"
                );
                Some(state)
            }
            Err(e) => {
                warn!("tool-result elision: stored state unreadable after write ({e})");
                None
            }
        },
        Ok(None) => None,
        Err(e) => {
            warn!("tool-result elision: could not persist the batch ({e}); not applying it");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::session_manager::SessionType;
    use rmcp::model::{CallToolRequestParams, ErrorCode, ErrorData};
    use rmcp::object;

    fn params() -> ElisionParams {
        ElisionParams {
            enabled: true,
            protect_tokens: 1_000,
            min_prunable_tokens: 1_000,
            min_chars: 400,
        }
    }

    fn request(id: &str, tool: &str) -> Message {
        Message::assistant().with_tool_request(
            id,
            Ok(CallToolRequestParams {
                name: tool.to_string().into(),
                arguments: Some(object!({"q": id})),
                meta: None,
                task: None,
            }),
        )
    }

    fn ok_result(text: &str) -> Result<CallToolResult, ErrorData> {
        Ok(CallToolResult {
            content: vec![Content::text(text)],
            structured_content: None,
            is_error: Some(false),
            meta: None,
        })
    }

    fn response(id: &str, text: &str) -> Message {
        Message::user().with_tool_response(id, ok_result(text))
    }

    /// A chat of `rounds` tool rounds, each result `chars` long, after one
    /// user prompt.
    fn chat(rounds: usize, chars: usize, tool: &str) -> Vec<Message> {
        let mut messages = vec![Message::user().with_text("analyse the cohort")];
        for i in 0..rounds {
            let id = format!("call_{i}");
            messages.push(request(&id, tool));
            messages.push(response(&id, &format!("row {i}\n{}", "x".repeat(chars))));
        }
        messages
    }

    fn text_of(message: &Message) -> String {
        message
            .content
            .iter()
            .filter_map(|c| match c {
                MessageContent::ToolResponse(r) => r.tool_result.as_ref().ok().map(|res| {
                    res.content
                        .iter()
                        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
                        .collect::<Vec<_>>()
                        .join("\n")
                }),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn entries_for(candidates: &[ElisionCandidate], dir: &Path) -> ToolResultElisions {
        ToolResultElisions {
            entries: save_candidates(candidates.to_vec(), "s1", dir, &dir.join("fallback"))
                .into_iter()
                .collect(),
        }
    }

    #[test]
    fn nothing_is_planned_below_the_prunable_minimum() {
        // 6 results of ~250 tokens: 1,000 protected, the ~500 left is under
        // the 1,000 minimum.
        let messages = chat(6, 1_000, "query");
        assert!(plan_elision(&messages, &ToolResultElisions::default(), &params()).is_empty());
    }

    #[test]
    fn a_batch_takes_every_eligible_older_result_and_spares_the_newest() {
        let messages = chat(12, 1_000, "query");
        let planned = plan_elision(&messages, &ToolResultElisions::default(), &params());
        let ids: Vec<&str> = planned.iter().map(|c| c.id.as_str()).collect();
        // Newest first: call_11..call_8 fill the 1,000-token protected window
        // (each ~252 tokens); the one that crosses it and everything older go.
        assert_eq!(ids.first(), Some(&"call_0"), "oldest first: {ids:?}");
        assert!(!ids.contains(&"call_11") && !ids.contains(&"call_10"));
        assert!(ids.contains(&"call_7"));
        assert!(planned.iter().map(|c| c.estimated_tokens).sum::<usize>() >= 1_000);
    }

    #[test]
    fn the_latest_round_is_never_elided_whatever_its_size() {
        let mut messages = chat(8, 1_000, "query");
        // The newest round's results are huge, well past the protected window.
        messages.push(request("big", "query"));
        messages.push(response("big", &"y".repeat(20_000)));
        let planned = plan_elision(&messages, &ToolResultElisions::default(), &params());
        assert!(!planned.is_empty());
        assert!(planned.iter().all(|c| c.id != "big"));
    }

    #[test]
    fn exempt_tools_small_results_and_failures_are_kept() {
        let mut messages = chat(10, 1_000, "todo__todo_update");
        messages.extend(chat(10, 1_000, "skills__loadSkill").into_iter().skip(1));
        messages.push(request("tiny", "query"));
        messages.push(response("tiny", "ok"));
        messages.push(request("failed", "query"));
        messages.push(Message::user().with_tool_response(
            "failed",
            Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "x".repeat(5_000),
                None,
            )),
        ));
        // Push the above out of the protected window.
        messages.extend(chat(6, 1_000, "query").into_iter().skip(1).map(|m| {
            let mut m = m;
            for c in m.content.iter_mut() {
                match c {
                    MessageContent::ToolRequest(r) => r.id = format!("late_{}", r.id),
                    MessageContent::ToolResponse(r) => r.id = format!("late_{}", r.id),
                    _ => {}
                }
            }
            m
        }));
        let planned = plan_elision(&messages, &ToolResultElisions::default(), &params());
        assert!(
            planned.iter().all(|c| !is_exempt_tool(&c.tool)),
            "{planned:?}"
        );
        assert!(planned.iter().all(|c| c.id != "tiny" && c.id != "failed"));
        assert!([
            "todo__todo_update",
            "skills__loadSkill",
            "todo_add",
            "workspace_watch"
        ]
        .iter()
        .all(|t| is_exempt_tool(t)));
        assert!(!is_exempt_tool("developer__shell"));
        assert!(!is_exempt_tool("code_execution__execute_code"));
    }

    #[test]
    fn an_error_result_keeps_its_first_lines() {
        let error = CallToolResult {
            content: vec![Content::text(format!(
                "Error: table missing\nline two\nline three\nline four\n{}",
                "z".repeat(3_000)
            ))],
            structured_content: None,
            is_error: Some(true),
            meta: None,
        };
        let mut messages = vec![request("e", "query")];
        messages.push(Message::user().with_tool_response("e", Ok(error)));
        messages.extend(chat(8, 1_000, "query"));
        let planned = plan_elision(&messages, &ToolResultElisions::default(), &params());
        let dir = tempfile::tempdir().unwrap();
        let state = entries_for(&planned, dir.path());
        let entry = state.entries.get("e").expect("the error was eligible");
        assert!(entry.is_error);
        let stub = render_stub(entry);
        assert!(stub.contains("Error: table missing\nline two\nline three"));
        assert!(!stub.contains("line four"));

        let applied = apply_elisions(Conversation::new_unvalidated(messages), &state);
        let MessageContent::ToolResponse(r) = &applied.messages()[1].content[0] else {
            panic!("still a tool response");
        };
        assert_eq!(r.tool_result.as_ref().unwrap().is_error, Some(true));
    }

    #[test]
    fn pairing_and_ids_survive_and_the_stub_points_at_a_readable_file() {
        let messages = chat(12, 1_000, "query");
        let planned = plan_elision(&messages, &ToolResultElisions::default(), &params());
        let dir = tempfile::tempdir().unwrap();
        let state = entries_for(&planned, dir.path());
        assert_eq!(state.entries.len(), planned.len());

        let applied = apply_elisions(Conversation::new_unvalidated(messages.clone()), &state);
        assert_eq!(
            applied.len(),
            messages.len(),
            "no message is added or removed"
        );
        // The same ids, in the same places: every request still has its response.
        for (before, after) in messages.iter().zip(applied.messages()) {
            let ids = |m: &Message| {
                m.content
                    .iter()
                    .filter_map(|c| match c {
                        MessageContent::ToolRequest(r) => Some(format!("req:{}", r.id)),
                        MessageContent::ToolResponse(r) => Some(format!("res:{}", r.id)),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(ids(before), ids(after));
            assert_eq!(before.role, after.role);
        }
        assert!(
            Conversation::new(applied.messages().clone()).is_ok(),
            "the stubbed conversation is still a valid one"
        );

        for (id, entry) in &state.entries {
            let original = messages
                .iter()
                .find(|m| {
                    m.content
                        .iter()
                        .any(|c| matches!(c, MessageContent::ToolResponse(r) if &r.id == id))
                })
                .unwrap();
            let saved = std::fs::read_to_string(&entry.path).expect("the stub's file exists");
            assert_eq!(saved, text_of(original), "the file holds the full text");
            let stubbed = applied
                .messages()
                .iter()
                .find(|m| {
                    m.content
                        .iter()
                        .any(|c| matches!(c, MessageContent::ToolResponse(r) if &r.id == id))
                })
                .unwrap();
            let stub = text_of(stubbed);
            assert!(stub.contains(&entry.path), "{stub}");
            assert!(stub.len() < 700, "a stub is short: {}", stub.len());
            assert!(entry.path.starts_with(&dir.path().display().to_string()));
        }
    }

    /// A chat whose oldest result is an offload preview naming `saved`.
    fn chat_with_offload_preview(saved: &Path) -> Vec<Message> {
        let preview = format!(
            "The `query` tool returned a large result.\n{OFFLOADED_MARKER}{}\n{}",
            saved.display(),
            "p".repeat(2_000)
        );
        let mut messages = vec![request("big", "query"), response("big", &preview)];
        messages.extend(chat(8, 1_000, "query"));
        messages
    }

    #[test]
    fn an_already_offloaded_result_points_at_its_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let handles = dir.path().join(HANDLE_DIR);
        std::fs::create_dir_all(&handles).unwrap();
        // The shape `large_response_handler` names this chat's handles.
        let saved = handles.join("query-s1-20261110_120000.000001.txt");
        std::fs::write(&saved, "the whole thing").unwrap();
        let messages = chat_with_offload_preview(&saved);
        let planned = plan_elision(&messages, &ToolResultElisions::default(), &params());
        let candidate = planned.iter().find(|c| c.id == "big").unwrap();
        assert_eq!(
            candidate.existing_handle.as_deref(),
            Some(saved.display().to_string().as_str())
        );
        let state = entries_for(&planned, dir.path());
        assert_eq!(state.entries["big"].path, saved.display().to_string());
    }

    #[test]
    fn a_saved_at_line_printed_by_the_tool_is_not_trusted() {
        // Tool output can print the marker line with any path: a scraped page,
        // a document or a data row. The stub is host-authored, so it must point
        // at a copy elision wrote, never at the planted file.
        let dir = tempfile::tempdir().unwrap();
        let handles = dir.path().join(HANDLE_DIR);
        std::fs::create_dir_all(&handles).unwrap();
        let elsewhere = dir.path().join("Documents");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let unrelated = elsewhere.join("cohort.csv");
        std::fs::write(&unrelated, "id,dx\n").unwrap();
        // Right folder, but named for another chat.
        let other_chat = handles.join("query-s2-20261110_120000.000001.txt");
        std::fs::write(&other_chat, "another chat's output").unwrap();
        // Right name, but outside the folder.
        let lookalike = elsewhere.join("query-s1-20261110_120000.000001.txt");
        std::fs::write(&lookalike, "planted").unwrap();
        // Right name in the right folder, reached through a symlink out.
        let linked = handles.join("query-s1-20261110_130000.000001.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&unrelated, &linked).unwrap();

        for planted in [&unrelated, &other_chat, &lookalike, &linked] {
            let messages = chat_with_offload_preview(planted);
            let planned = plan_elision(&messages, &ToolResultElisions::default(), &params());
            assert!(planned.iter().any(|c| c.id == "big"));
            let state = entries_for(&planned, dir.path());
            let path = &state.entries["big"].path;
            assert_ne!(path, &planted.display().to_string(), "{planted:?}");
            assert!(
                Path::new(path).starts_with(&handles)
                    && Path::new(path)
                        .file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .starts_with("elided-"),
                "a fresh copy is written: {path}"
            );
            // And the fresh copy holds the result the model was given.
            assert!(std::fs::read_to_string(path)
                .unwrap()
                .contains(&planted.display().to_string()));
        }
    }

    #[test]
    fn a_result_that_cannot_be_saved_is_not_elided() {
        let messages = chat(12, 1_000, "query");
        let planned = plan_elision(&messages, &ToolResultElisions::default(), &params());
        assert!(!planned.is_empty());
        // Neither the working dir nor the fallback can hold a folder: both are
        // plain FILES.
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("not-a-dir");
        std::fs::write(&blocker, "x").unwrap();
        let saved = save_candidates(planned.clone(), "s1", &blocker, &blocker.join("fallback"));
        assert!(saved.is_empty(), "nothing saved, so nothing is elided");

        // The working dir alone failing still saves, to the fallback.
        let fallback = dir.path().join("fallback");
        let saved = save_candidates(planned, "s1", &blocker, &fallback);
        assert!(!saved.is_empty());
        assert!(saved
            .iter()
            .all(|(_, e)| e.path.starts_with(&fallback.display().to_string())
                && Path::new(&e.path).is_file()));

        // And a state without an entry leaves every result alone.
        let applied = apply_elisions(
            Conversation::new_unvalidated(messages.clone()),
            &ToolResultElisions::default(),
        );
        assert_eq!(applied.messages(), &messages);
    }

    #[test]
    fn stubs_are_byte_identical_between_batches() {
        let mut messages = chat(12, 1_000, "query");
        let planned = plan_elision(&messages, &ToolResultElisions::default(), &params());
        let dir = tempfile::tempdir().unwrap();
        let state = entries_for(&planned, dir.path());

        let first = apply_elisions(Conversation::new_unvalidated(messages.clone()), &state);
        // Two more rounds arrive; nothing new is due, so the next request is
        // the previous one plus the new tail, byte for byte.
        messages.push(request("next_a", "query"));
        messages.push(response("next_a", &"n".repeat(500)));
        assert!(plan_elision(&messages, &state, &params()).is_empty());
        let second = apply_elisions(Conversation::new_unvalidated(messages.clone()), &state);
        let encode = |c: &Conversation| {
            c.messages()
                .iter()
                .map(|m| serde_json::to_string(&m.content).unwrap())
                .collect::<Vec<_>>()
        };
        let (a, b) = (encode(&first), encode(&second));
        assert_eq!(a[..], b[..a.len()], "the earlier prefix did not move");
    }

    #[test]
    fn a_second_pass_adds_nothing_until_enough_new_output_ages() {
        let mut messages = chat(12, 1_000, "query");
        let dir = tempfile::tempdir().unwrap();
        let first = plan_elision(&messages, &ToolResultElisions::default(), &params());
        let state = entries_for(&first, dir.path());
        assert!(
            plan_elision(&messages, &state, &params()).is_empty(),
            "idempotent"
        );
        // Many more rounds: older ones leave the protected window again.
        for i in 0..10 {
            let id = format!("more_{i}");
            messages.push(request(&id, "query"));
            messages.push(response(&id, &"m".repeat(1_000)));
        }
        let second = plan_elision(&messages, &state, &params());
        assert!(!second.is_empty());
        assert!(second.iter().all(|c| !state.entries.contains_key(&c.id)));
    }

    #[test]
    fn the_kill_switch_plans_nothing() {
        let messages = chat(40, 1_000, "query");
        let off = ElisionParams {
            enabled: false,
            ..params()
        };
        assert!(plan_elision(&messages, &ToolResultElisions::default(), &off).is_empty());
        assert!(ElisionParams::default().enabled);
    }

    #[test]
    fn content_for_the_person_alone_is_kept_and_only_model_text_is_stubbed() {
        let mut person_only = Content::text("shown to the person");
        person_only = person_only.with_audience(vec![Role::User]);
        let result = CallToolResult {
            content: vec![Content::text("m".repeat(3_000)), person_only.clone()],
            structured_content: None,
            is_error: None,
            meta: None,
        };
        let mut messages = vec![request("mixed", "query")];
        messages.push(Message::user().with_tool_response("mixed", Ok(result)));
        messages.extend(chat(8, 1_000, "query"));
        let planned = plan_elision(&messages, &ToolResultElisions::default(), &params());
        let candidate = planned.iter().find(|c| c.id == "mixed").unwrap();
        assert!(!candidate.text.contains("shown to the person"));
        let dir = tempfile::tempdir().unwrap();
        let state = entries_for(&planned, dir.path());
        let applied = apply_elisions(Conversation::new_unvalidated(messages), &state);
        let MessageContent::ToolResponse(r) = &applied.messages()[1].content[0] else {
            panic!()
        };
        let content = &r.tool_result.as_ref().unwrap().content;
        assert_eq!(content.len(), 2);
        assert!(content[0]
            .as_text()
            .unwrap()
            .text
            .contains("[Earlier tool result"));
        assert_eq!(content[1], person_only);
    }

    #[tokio::test]
    async fn the_whole_step_persists_a_batch_and_never_touches_the_stored_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let sm = crate::session::SessionManager::new(dir.path().to_path_buf());
        let session = sm
            .create_session(dir.path().to_path_buf(), "e".into(), SessionType::User)
            .await
            .unwrap();
        // Results of ~2,000 characters (~500 estimated tokens), over the
        // 1,500-character minimum.
        for message in chat(100, 2_000, "query") {
            sm.add_message(&session.id, &message).await.unwrap();
        }
        let stored = sm
            .get_session(&session.id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();

        // The default thresholds: ~50k tokens of results, 40k protected, and
        // the ~10k left is under the 25k minimum, so nothing yet.
        let sent =
            elide_stale_tool_results(&sm, &session.id, stored.clone(), dir.path(), true).await;
        assert_eq!(sent.messages(), stored.messages());

        let mut longer = stored.clone();
        for message in chat(150, 2_000, "query").into_iter().skip(1) {
            let mut message = message;
            for c in message.content.iter_mut() {
                match c {
                    MessageContent::ToolRequest(r) => r.id = format!("b_{}", r.id),
                    MessageContent::ToolResponse(r) => r.id = format!("b_{}", r.id),
                    _ => {}
                }
            }
            sm.add_message(&session.id, &message).await.unwrap();
            longer.push(message);
        }
        let due = plan_elision(
            longer.messages(),
            &ToolResultElisions::default(),
            &ElisionParams::from_config(),
        );
        assert!(!due.is_empty(), "a batch is due at the default thresholds");
        // No tool can read a saved file back: nothing is elided.
        let unread =
            elide_stale_tool_results(&sm, &session.id, longer.clone(), dir.path(), false).await;
        assert_eq!(unread.messages(), longer.messages());
        let first =
            elide_stale_tool_results(&sm, &session.id, longer.clone(), dir.path(), true).await;
        assert_ne!(first.messages(), longer.messages(), "a batch ran");
        let persisted = sm
            .get_extension_state(&session.id, "tool_elision", "v1")
            .await
            .unwrap()
            .expect("the batch is persisted");
        let state = ToolResultElisions::from_value(&persisted).unwrap();
        assert!(!state.entries.is_empty());

        // The next request, with no new output, is byte-identical.
        let again =
            elide_stale_tool_results(&sm, &session.id, longer.clone(), dir.path(), true).await;
        assert_eq!(again.messages(), first.messages());

        // The transcript the person sees is untouched.
        let after = sm
            .get_session(&session.id, true)
            .await
            .unwrap()
            .conversation
            .unwrap();
        assert_eq!(after.len(), longer.len());
        for (a, b) in after.messages().iter().zip(longer.messages()) {
            assert_eq!(a.content, b.content);
        }
    }

    /// The compaction check measures what the model is SENT. The provider's
    /// figure counts stubs; judged against the full text, the floor check
    /// would distrust it, estimate a request the model never got, and compact
    /// a chat whose real context is small.
    #[tokio::test]
    async fn the_compaction_check_measures_the_elided_request() {
        let dir = tempfile::tempdir().unwrap();
        let sm = crate::session::SessionManager::new(dir.path().to_path_buf());
        let created = sm
            .create_session(dir.path().to_path_buf(), "c".into(), SessionType::User)
            .await
            .unwrap();
        // ~800k characters of tool output: about 100k tokens at the floor's
        // most generous 8 bytes a token, far more at a real rate.
        let messages = chat(200, 4_000, "query");
        let conversation = Conversation::new_unvalidated(messages.clone());
        let provider = crate::context_mgmt::tests::MockProvider::new(
            Message::assistant().with_text("unused"),
            128_000,
        );

        let mut session = sm.get_session(&created.id, false).await.unwrap();
        // What the provider reported for the ELIDED request.
        session.total_tokens = Some(60_000);
        assert!(
            crate::context_mgmt::check_if_compaction_needed(
                &provider,
                &conversation,
                None,
                &session,
                None
            )
            .await
            .unwrap(),
            "without the elisions the 60k figure looks truncated and the full text is counted"
        );

        // Every result but the newest few is elided.
        let planned = plan_elision(&messages, &ToolResultElisions::default(), &params());
        let state = entries_for(&planned, dir.path());
        assert!(state.entries.len() > 150);
        let value = state.to_value().unwrap();
        sm.update_extension_state(
            &created.id,
            ToolResultElisions::EXTENSION_NAME,
            ToolResultElisions::VERSION,
            move |_| Ok(value),
        )
        .await
        .unwrap();
        let mut session = sm.get_session(&created.id, false).await.unwrap();
        session.total_tokens = Some(60_000);
        assert!(
            !crate::context_mgmt::check_if_compaction_needed(
                &provider,
                &conversation,
                None,
                &session,
                None
            )
            .await
            .unwrap(),
            "with them, the reported 60k is the request the model got"
        );
    }
}
