pub mod computer_use;
pub mod global_memory;
pub mod knowledge_delete;
pub mod patterns;
pub mod policy;
pub mod security_inspector;
pub mod sensitive_ops;
pub mod session_store;

use crate::config::Config;
use crate::conversation::message::ToolRequest;
use rmcp::model::CallToolRequestParams;

/// A catastrophic-command hard block. These fire regardless of permission mode
/// or any config flag and cannot be bypassed by config.
#[derive(Debug, Clone)]
pub struct CatastrophicBlock {
    pub tool_request_id: String,
    pub rule_name: &'static str,
    /// A self-contained, user-facing message naming the rule that fired.
    pub message: String,
}

/// Argument keys that carry an executable command string (scanned on any tool).
const COMMAND_ARG_KEYS: &[&str] = &["command", "cmd", "commands", "shell_command"];

/// Substrings in a tool name that mark it as a shell/command executor (scan all
/// of its string arguments, not just the command-bearing keys above).
const SHELL_TOOL_HINTS: &[&str] = &["shell", "bash", "terminal"];

/// Object-argument keys whose string (or string-array) value is a filesystem
/// path. Also matched by [`is_path_argument_key`]: any key ending in `_path`
/// (`dest_path`, `src_path`, `image_path`, `workflow_path`), and the plural
/// `paths`.
///
/// This list lives here, rather than in one of its callers, because more than
/// one security check has to answer "is this argument a path?" and they were
/// answering it differently. [`sensitive_ops`] classifies the blast radius of a
/// path a tool is about to write; [`global_memory`] refuses a path that names
/// the machine-wide memory store. A key missing from one list but present in
/// the other is a gap in exactly one of them, silently.
///
/// Two lists elsewhere still disagree with this one and are deliberately left
/// alone: `agents::code_execution_extension` and `agents::tool_dispatch_limits`
/// each carry a shorter one for a different purpose. Folding those in would
/// change what they escalate, which is not this module's decision to make.
pub(crate) const PATH_ARG_KEYS: &[&str] = &[
    "path",
    "file_path",
    "filepath",
    "filename",
    "file",
    "target",
    "target_path",
    "dest",
    "destination",
    "output_path",
    "output",
    "to",
    "new_path",
];

/// Does `key` name an argument whose value is a filesystem path?
///
/// Case-insensitive, and matches the `_path` suffix and the plural `paths` on
/// top of [`PATH_ARG_KEYS`].
pub(crate) fn is_path_argument_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    PATH_ARG_KEYS.iter().any(|candidate| key == *candidate)
        || key.ends_with("_path")
        || key == "paths"
}

/// Does `key` name an argument whose value is a shell command line?
pub(crate) fn is_command_argument_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    COMMAND_ARG_KEYS.iter().any(|candidate| key == *candidate)
}

/// A production route that dispatches tool calls without passing them through
/// the agent loop — and therefore without passing them through **any**
/// [`crate::tool_inspection::ToolInspector`] (issue #63 review, finding 3).
///
/// This enumeration lives here, not in one of its callers, for the reason
/// [`PATH_ARG_KEYS`] does: more than one security check has to answer "which
/// doors bypass the inspector chain?", and two copies of the answer are a gap in
/// exactly one of them, silently. [`global_memory`] refuses machine-wide memory
/// operations at these doors; [`session_store`] refuses reads of the transcript
/// database at them. A third boundary added to the tree is one row here and a
/// red build at every gate that has not decided about it, rather than a hole in
/// whichever gate nobody remembered.
///
/// ⚠ **The refusal at these doors is not a worse version of the inspector — it
/// is a stronger one, and that is the whole point.** An inspector sees the tool
/// call a model *wrote*, so a path or a flag assembled at runtime walks past it
/// (`const p = home + "/.config/…"`). A boundary sees the dispatched name and
/// the already-evaluated arguments; there is nothing left to compute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UninspectedBoundary {
    /// The tool calls a script makes from inside `execute_code`. The JS host
    /// hands them straight to `ExtensionManager::dispatch_tool_call`.
    ExecuteCodeScript,
    /// `POST /agent/call_tool`, which does the same from an HTTP handler.
    AgentCallToolRoute,
}

/// Owner of the two always-on command controls: the non-bypassable
/// catastrophic-command denylist (BR-20) and the enable gate for the auditable
/// command policy engine (BR-21).
#[derive(Debug)]
pub struct SecurityManager;

impl SecurityManager {
    pub fn new() -> Self {
        Self
    }

    /// Whether the auditable command policy engine (BR-21) governs commands.
    ///
    /// On by default; `SECURITY_COMMAND_POLICY=off` disables it for a nervous rollout, leaving
    /// only the non-bypassable BR-20 catastrophic floor. Any other value (e.g.
    /// the default `enforce`) keeps it on.
    pub fn is_command_policy_enabled(&self) -> bool {
        let config = Config::global();

        match config.get_param::<String>("SECURITY_COMMAND_POLICY") {
            Ok(v) => !v.trim().eq_ignore_ascii_case("off"),
            Err(_) => true,
        }
    }

    /// Scan tool calls against the always-on catastrophic-command denylist.
    ///
    /// This runs unconditionally — independent of every config flag and
    /// of the session's permission mode — so that a handful of unrecoverable
    /// commands (e.g. `rm -rf /`, disk wipes, fork bombs) are hard-blocked even
    /// in `Auto` mode. Blocks are returned as `Deny` decisions by the security
    /// inspector and cannot be overridden by the user.
    pub fn catastrophic_blocks(&self, tool_requests: &[ToolRequest]) -> Vec<CatastrophicBlock> {
        let mut blocks = Vec::new();
        for tool_request in tool_requests {
            let Ok(tool_call) = &tool_request.tool_call else {
                continue;
            };
            let Some(text) = command_text(tool_call) else {
                continue;
            };
            if let Some(rule) = patterns::match_catastrophic_command(&text) {
                let message = format!(
                    "Blocked by Biorouter's always-on catastrophic-command denylist \
                     (rule: {}): {}. This safety rule cannot be bypassed by permission mode.",
                    rule.name, rule.description
                );
                tracing::warn!(
                    counter.biorouter.catastrophic_command_blocked = 1,
                    tool_name = %tool_call.name,
                    tool_request_id = %tool_request.id,
                    rule = rule.name,
                    "Catastrophic command hard-blocked (non-bypassable)"
                );
                blocks.push(CatastrophicBlock {
                    tool_request_id: tool_request.id.clone(),
                    rule_name: rule.name,
                    message,
                });
            }
        }
        blocks
    }
}

impl Default for SecurityManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract the command text to screen from a tool call: the string values of
/// any command-bearing argument, plus every string argument when the tool name
/// looks like a shell/command executor. Returns `None` when there is nothing
/// command-like to scan (so file contents written by an editor are not screened).
fn command_text(tool_call: &CallToolRequestParams) -> Option<String> {
    let args = tool_call.arguments.as_ref()?;
    command_text_from(&tool_call.name, args)
}

/// Same as [`command_text`] but from a tool name + its argument map, so the
/// policy engine (which is handed `tool_name` + a `serde_json::Value`) can reuse
/// the exact command-extraction rules the catastrophic denylist uses.
pub(crate) fn command_text_from(
    name: &str,
    args: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let name_lc = name.to_ascii_lowercase();
    let shell_like = SHELL_TOOL_HINTS.iter().any(|hint| name_lc.contains(hint));

    let mut parts: Vec<String> = Vec::new();
    for (key, value) in args.iter() {
        let key_lc = key.to_ascii_lowercase();
        let is_command_key = COMMAND_ARG_KEYS.iter().any(|k| key_lc == *k);
        if shell_like || is_command_key {
            collect_strings(value, &mut parts);
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

/// Recursively collect every string scalar in a JSON value (handles arrays of
/// commands and nested objects).
fn collect_strings(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) => out.push(s.clone()),
        serde_json::Value::Array(items) => {
            for item in items {
                collect_strings(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values() {
                collect_strings(v, out);
            }
        }
        _ => {}
    }
}
