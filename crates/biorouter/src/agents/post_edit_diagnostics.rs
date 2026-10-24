//! Auto post-edit diagnostics feedback loop (BR-47).
//!
//! `text_editor` writes never triggered a syntax check: the model learned a file
//! was broken only if it *chose* to run tests. The developer extension's own
//! tree-sitter analyzer can parse the just-written file and report its ERROR /
//! MISSING nodes essentially for free (see
//! [`biorouter_mcp::developer::analyze::diagnostics`]), but nothing wired that
//! capability into the edit path.
//!
//! The built-in editor computes diagnostics through its bounded file capability.
//! The dispatcher replaces the reserved data with a short-lived, one-use receipt;
//! the agent consumes it without reopening a pathname after authorization ends.
//! The reflection counter bounds corrective context so invalid syntax cannot
//! wedge the turn.
//!
//! It rides the BR-19 PostToolUse seam: the diagnostics are computed at the same
//! turn boundary where PostToolUse hooks inject their `additionalContext`, and
//! the reflection counter mirrors the `POST_TOOL_HOOK_BLOCK_CAP` bound so an
//! always-broken file behaves exactly like an always-blocking hook — capped, then
//! delivered as-is.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use rand::RngCore;
use rmcp::model::CallToolResult;
use serde_json::Value;

use crate::config::Config;
use crate::conversation::message::ToolRequest;

/// Default cap on how many times per reply a broken edit is reflected back to the
/// model. Aider's `max_reflections` is 3; a file still broken after three
/// corrective nudges in one reply will not be fixed by a fourth, and the model can
/// still act on the tool result it can already see.
pub const DEFAULT_MAX_REFLECTIONS: u32 = 3;

/// BR-47 policy, resolved once per reply (config reads touch the filesystem).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostEditDiagnosticsConfig {
    /// Master switch. Off = the exact pre-BR-47 behaviour (no post-edit check).
    pub enabled: bool,
    /// Per-reply reflection cap. `0` also disables injection.
    pub max_reflections: u32,
}

impl Default for PostEditDiagnosticsConfig {
    fn default() -> Self {
        Self {
            // Default OFF: merging this branch changes no runtime behaviour until a
            // user opts in (BIOROUTER_POST_EDIT_DIAGNOSTICS=1). Per the campaign's
            // signed-off decision that all new capabilities ship default-off.
            enabled: false,
            max_reflections: DEFAULT_MAX_REFLECTIONS,
        }
    }
}

impl PostEditDiagnosticsConfig {
    /// Inert (pre-BR-47 behaviour).
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            max_reflections: 0,
        }
    }

    /// Resolve from config. Keys:
    ///
    /// * `BIOROUTER_POST_EDIT_DIAGNOSTICS` (bool) — master switch.
    /// * `BIOROUTER_POST_EDIT_DIAGNOSTICS_MAX_REFLECTIONS` (u32) — the per-reply cap.
    pub fn from_config() -> Self {
        Self::from(Config::global())
    }

    pub fn from(config: &Config) -> Self {
        let defaults = Self::default();
        let enabled = config
            .get_param::<bool>("BIOROUTER_POST_EDIT_DIAGNOSTICS")
            .unwrap_or(defaults.enabled);
        let max_reflections = config
            .get_param::<u32>("BIOROUTER_POST_EDIT_DIAGNOSTICS_MAX_REFLECTIONS")
            .unwrap_or(defaults.max_reflections);
        Self {
            enabled,
            max_reflections,
        }
    }

    /// Whether any injection can happen at all.
    pub fn is_active(&self) -> bool {
        self.enabled && self.max_reflections > 0
    }
}

/// The file a successful `text_editor` write changed, resolved against
/// `working_dir`.
///
/// `None` for a non-write command (`view` / `undo_edit`), a tool that is not
/// `text_editor`, or a call with no usable path. Only `write` / `str_replace` /
/// `insert` change file *content* in a way a fresh parse can check.
pub fn edited_path_from_tool_call(
    tool_name: &str,
    arguments: &serde_json::Map<String, Value>,
    working_dir: &Path,
) -> Option<PathBuf> {
    if !is_text_editor(tool_name) {
        return None;
    }
    let command = arguments.get("command").and_then(Value::as_str)?;
    if !matches!(command, "write" | "str_replace" | "insert") {
        return None;
    }
    // `file_path` is the developer tool's documented alias for `path`.
    let raw = arguments
        .get("path")
        .or_else(|| arguments.get("file_path"))
        .and_then(Value::as_str)?;
    if raw.trim().is_empty() {
        return None;
    }
    Some(resolve_path(raw, working_dir))
}

/// Convenience for the agent loop: pull the edited path straight off a
/// [`ToolRequest`]. `None` unless the request is a well-formed `text_editor`
/// write.
pub fn edited_path_from_request(request: &ToolRequest, working_dir: &Path) -> Option<PathBuf> {
    let tool_call = request.tool_call.as_ref().ok()?;
    let arguments = tool_call.arguments.as_ref()?;
    edited_path_from_tool_call(tool_call.name.as_ref(), arguments, working_dir)
}

fn is_text_editor(tool_name: &str) -> bool {
    // Extension tools are namespaced `<server>__<tool>` (e.g. `developer__text_editor`).
    tool_name == "text_editor" || tool_name.ends_with("__text_editor")
}

fn resolve_path(raw: &str, working_dir: &Path) -> PathBuf {
    let expanded = shellexpand::tilde(raw);
    let path = Path::new(expanded.as_ref());
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        working_dir.join(path)
    }
}

/// What to do with a batch of freshly-checked edits, given the running
/// reflection count. Keeps the increment / reset / cap policy out of the reply
/// loop so it can be unit-tested on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReflectionOutcome {
    /// At least one edited file is still broken and the budget is not spent:
    /// inject the diagnostics and advance the counter to `reflections + 1`.
    Inject { next: u32 },
    /// Every edited file parsed clean (or there was nothing dirty): a genuine fix
    /// restores the budget, so the counter resets to 0.
    Reset,
    /// A file is still broken but the per-reply cap is reached: deliver the result
    /// as-is and leave the counter where it is, so the turn is never wedged.
    Capped,
}

/// Resolve the reflection policy for one batch of post-edit checks.
///
/// `any_dirty` is whether any edited file produced diagnostics this batch;
/// `reflections` is how many diagnostics injections have already happened this
/// reply; `cap` is [`PostEditDiagnosticsConfig::max_reflections`].
pub fn next_reflection(any_dirty: bool, reflections: u32, cap: u32) -> ReflectionOutcome {
    if !any_dirty {
        ReflectionOutcome::Reset
    } else if reflections < cap {
        ReflectionOutcome::Inject {
            next: reflections + 1,
        }
    } else {
        ReflectionOutcome::Capped
    }
}

/// One file's diagnostics, ready to render into the corrective message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiagnostics {
    /// Path as it should read to the model (usually the raw argument the model sent).
    pub path: String,
    /// One rendered `line L:C: message` per diagnostic.
    pub lines: Vec<String>,
}

pub(crate) const BOUNDED_DIAGNOSTICS_META_KEY: &str = "biorouter-bounded-edit-diagnostics";
pub(crate) const DIAGNOSTICS_RECEIPT_META_KEY: &str = "biorouter-bounded-edit-receipt";
const RECEIPT_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_RECEIPTS: usize = 256;
const MAX_DIAGNOSTIC_FILES: usize = 128;
const MAX_DIAGNOSTIC_LINES: usize = 128;
const MAX_DIAGNOSTIC_BYTES: usize = 256 * 1024;

struct DiagnosticsReceipt {
    session: String,
    tool_name: String,
    arguments: blake3::Hash,
    files: Vec<FileDiagnostics>,
    issued: Instant,
}

#[derive(Default)]
struct ReceiptRegistry {
    entries: HashMap<String, DiagnosticsReceipt>,
}

impl ReceiptRegistry {
    fn prune(&mut self, now: Instant) {
        self.entries
            .retain(|_, entry| now.saturating_duration_since(entry.issued) < RECEIPT_TTL);
    }

    fn insert(&mut self, entry: DiagnosticsReceipt, now: Instant) -> String {
        self.prune(now);
        if self.entries.len() >= MAX_RECEIPTS {
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.issued)
                .map(|(key, _)| key.clone())
            {
                self.entries.remove(&oldest);
            }
        }
        let mut random = [0_u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut random);
        let receipt = format!("{:032x}", u128::from_ne_bytes(random));
        self.entries.insert(receipt.clone(), entry);
        receipt
    }

    fn consume(
        &mut self,
        receipt: &str,
        session: &str,
        tool_name: &str,
        arguments: blake3::Hash,
        now: Instant,
    ) -> Option<Vec<FileDiagnostics>> {
        self.prune(now);
        let entry = self.entries.get(receipt)?;
        if entry.session != session || entry.tool_name != tool_name || entry.arguments != arguments
        {
            return None;
        }
        self.entries.remove(receipt).map(|entry| entry.files)
    }
}

static DIAGNOSTICS_RECEIPTS: LazyLock<Mutex<ReceiptRegistry>> =
    LazyLock::new(|| Mutex::new(ReceiptRegistry::default()));

fn parse_bounded_diagnostics(raw: Value) -> Option<Vec<FileDiagnostics>> {
    let Value::Array(files) = raw else {
        return None;
    };
    if files.len() > MAX_DIAGNOSTIC_FILES {
        return None;
    }
    let mut bytes = 0_usize;
    let mut parsed = Vec::with_capacity(files.len());
    for file in files {
        let Value::Object(mut fields) = file else {
            return None;
        };
        if fields.len() != 2 {
            return None;
        }
        let Value::String(path) = fields.remove("path")? else {
            return None;
        };
        let Value::Array(raw_lines) = fields.remove("lines")? else {
            return None;
        };
        if path.is_empty()
            || path.len() > 4096
            || path.contains('\0')
            || raw_lines.len() > MAX_DIAGNOSTIC_LINES
        {
            return None;
        }
        bytes = bytes.checked_add(path.len())?;
        let mut lines = Vec::with_capacity(raw_lines.len());
        for line in raw_lines {
            let Value::String(line) = line else {
                return None;
            };
            if line.len() > 2048 || line.contains('\0') {
                return None;
            }
            bytes = bytes.checked_add(line.len())?;
            if bytes > MAX_DIAGNOSTIC_BYTES {
                return None;
            }
            lines.push(line);
        }
        if bytes > MAX_DIAGNOSTIC_BYTES {
            return None;
        }
        parsed.push(FileDiagnostics { path, lines });
    }
    Some(parsed)
}

fn arguments_fingerprint(
    arguments: Option<&serde_json::Map<String, Value>>,
) -> Option<blake3::Hash> {
    let serialized = serde_json::to_vec(&arguments).ok()?;
    if serialized.len() > 8 * 1024 * 1024 {
        return None;
    }
    Some(blake3::hash(&serialized))
}

pub(crate) fn mint_bounded_diagnostics_receipt(
    session: &str,
    tool_name: &str,
    arguments: Option<&serde_json::Map<String, Value>>,
    raw: Value,
) -> Option<String> {
    if session.is_empty() || session.len() > 1024 || tool_name.is_empty() || tool_name.len() > 4096
    {
        return None;
    }
    let files = parse_bounded_diagnostics(raw)?;
    let arguments = arguments_fingerprint(arguments)?;
    let now = Instant::now();
    let entry = DiagnosticsReceipt {
        session: session.to_owned(),
        tool_name: tool_name.to_owned(),
        arguments,
        files,
        issued: now,
    };
    Some(
        DIAGNOSTICS_RECEIPTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(entry, now),
    )
}

pub(crate) fn consume_bounded_diagnostics_receipt(
    session: &str,
    tool_name: &str,
    arguments: Option<&serde_json::Map<String, Value>>,
    receipt: &str,
) -> Option<Vec<FileDiagnostics>> {
    let arguments = arguments_fingerprint(arguments)?;
    DIAGNOSTICS_RECEIPTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .consume(receipt, session, tool_name, arguments, Instant::now())
}

pub(crate) fn take_diagnostics_receipt(result: &mut CallToolResult) -> Option<String> {
    let meta = result.meta.as_mut()?;
    meta.0.remove(BOUNDED_DIAGNOSTICS_META_KEY);
    meta.0
        .remove(DIAGNOSTICS_RECEIPT_META_KEY)?
        .as_str()
        .map(str::to_owned)
}

/// Frame a batch of post-edit diagnostics as agent-visible corrective context.
///
/// The wrapper tag mirrors the hook-context framing so the model treats it as a
/// self-contained signal, and it is explicit that this is a *syntax* check (not a
/// type/lint pass) and that a suspected false positive should be noted and passed,
/// not looped on — the reflection cap is the hard backstop, this is the soft one.
pub fn frame_post_edit_diagnostics(files: &[FileDiagnostics]) -> String {
    let mut body = String::new();
    for file in files {
        body.push_str(&format!(
            "`{}`: {} syntax issue(s) after your edit:\n",
            file.path,
            file.lines.len()
        ));
        for line in &file.lines {
            body.push_str(&format!("  - {line}\n"));
        }
    }
    format!(
        "<post-edit-diagnostics>\n\
         An automatic syntax check ran on the file(s) you just edited and found problems. \
         This is a tree-sitter parse check \u{2014} it catches broken syntax (unbalanced \
         brackets, missing tokens), not type, import, or logic errors.\n\
         {body}\
         Fix these before continuing. If a flagged line is actually valid syntax (a false \
         positive), say so in one line and move on \u{2014} do not loop on it.\n\
         </post-edit-diagnostics>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    fn args(v: Value) -> serde_json::Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn diagnostics_receipt_is_bound_to_session_tool_and_full_arguments_and_one_use() {
        let session = Uuid::new_v4().to_string();
        let arguments =
            args(json!({"command":"write", "path":"/synthetic/a.py", "file_text":"bad syntax"}));
        let raw = json!([{"path":"/synthetic/a.py", "lines":["line 1:1: unexpected syntax"]}]);
        let receipt = mint_bounded_diagnostics_receipt(
            &session,
            "developer__text_editor",
            Some(&arguments),
            raw,
        )
        .unwrap();
        assert!(consume_bounded_diagnostics_receipt(
            "other-session",
            "developer__text_editor",
            Some(&arguments),
            &receipt
        )
        .is_none());
        assert!(consume_bounded_diagnostics_receipt(
            &session,
            "frontend__text_editor",
            Some(&arguments),
            &receipt
        )
        .is_none());
        let changed =
            args(json!({"command":"write", "path":"/synthetic/a.py", "file_text":"different"}));
        assert!(consume_bounded_diagnostics_receipt(
            &session,
            "developer__text_editor",
            Some(&changed),
            &receipt
        )
        .is_none());
        let files = consume_bounded_diagnostics_receipt(
            &session,
            "developer__text_editor",
            Some(&arguments),
            &receipt,
        )
        .unwrap();
        assert_eq!(files[0].lines, ["line 1:1: unexpected syntax"]);
        assert!(consume_bounded_diagnostics_receipt(
            &session,
            "developer__text_editor",
            Some(&arguments),
            &receipt
        )
        .is_none());
    }

    #[test]
    fn clean_receipt_preserves_reset_and_invalid_payloads_cannot_mint() {
        let session = Uuid::new_v4().to_string();
        let receipt =
            mint_bounded_diagnostics_receipt(&session, "developer__text_editor", None, json!([]))
                .unwrap();
        let files =
            consume_bounded_diagnostics_receipt(&session, "developer__text_editor", None, &receipt)
                .unwrap();
        assert!(files.is_empty());
        assert_eq!(
            next_reflection(!files.is_empty(), 2, 3),
            ReflectionOutcome::Reset
        );
        for raw in [
            json!({"path":"unexpected"}),
            json!([{"path":"/file", "lines":[1]}]),
            json!([{"path":"/file", "lines":[], "source":"unapproved"}]),
            json!([{"path":"/file", "lines":["x".repeat(2049)]}]),
            json!([{"path":"/file", "lines":vec!["line"; MAX_DIAGNOSTIC_LINES + 1]}]),
            json!(vec![
                json!({"path":"/file", "lines":[]});
                MAX_DIAGNOSTIC_FILES + 1
            ]),
        ] {
            assert!(mint_bounded_diagnostics_receipt(
                &session,
                "developer__text_editor",
                None,
                raw
            )
            .is_none());
        }
        assert!(consume_bounded_diagnostics_receipt(
            &session,
            "developer__text_editor",
            None,
            "forged-receipt"
        )
        .is_none());
    }

    #[test]
    fn receipt_registry_expires_and_evicts_old_entries_without_sleeping() {
        let now = Instant::now();
        let fingerprint = arguments_fingerprint(None).unwrap();
        let entry = |issued| DiagnosticsReceipt {
            session: "session".to_owned(),
            tool_name: "developer__text_editor".to_owned(),
            arguments: fingerprint,
            files: vec![],
            issued,
        };
        let mut registry = ReceiptRegistry::default();
        let expired = registry.insert(entry(now), now);
        assert!(registry
            .consume(
                &expired,
                "session",
                "developer__text_editor",
                fingerprint,
                now + RECEIPT_TTL
            )
            .is_none());
        let first = registry.insert(entry(now), now);
        for index in 1..=MAX_RECEIPTS {
            let issued = now + Duration::from_nanos(index as u64);
            registry.insert(entry(issued), issued);
        }
        assert_eq!(registry.entries.len(), MAX_RECEIPTS);
        assert!(!registry.entries.contains_key(&first));
    }

    #[test]
    fn reserved_metadata_is_removed_before_model_or_storage_serialization() {
        let mut result = CallToolResult::success(vec![]);
        let mut meta = rmcp::model::Meta::new();
        meta.0.insert(
            BOUNDED_DIAGNOSTICS_META_KEY.to_owned(),
            json!([{"path":"/private", "lines":["synthetic diagnostic"]}]),
        );
        meta.0
            .insert(DIAGNOSTICS_RECEIPT_META_KEY.to_owned(), json!("forged"));
        meta.0.insert("unrelated".to_owned(), json!(true));
        result.meta = Some(meta);
        assert_eq!(
            take_diagnostics_receipt(&mut result).as_deref(),
            Some("forged")
        );
        let serialized = serde_json::to_string(&result).unwrap();
        assert!(!serialized.contains("synthetic diagnostic"));
        assert!(!serialized.contains("forged"));
        assert!(serialized.contains("unrelated"));
    }

    #[test]
    fn write_command_yields_absolute_path() {
        let a = args(json!({"command": "write", "path": "/repo/src/main.rs", "file_text": "x"}));
        let p = edited_path_from_tool_call("developer__text_editor", &a, Path::new("/work"));
        assert_eq!(p, Some(PathBuf::from("/repo/src/main.rs")));
    }

    #[test]
    fn relative_path_resolves_against_working_dir() {
        let a = args(json!({"command": "str_replace", "path": "src/main.rs"}));
        let p = edited_path_from_tool_call("text_editor", &a, Path::new("/work"));
        assert_eq!(p, Some(PathBuf::from("/work/src/main.rs")));
    }

    #[test]
    fn file_path_alias_is_accepted() {
        let a = args(json!({"command": "insert", "file_path": "/repo/a.py"}));
        let p = edited_path_from_tool_call("developer__text_editor", &a, Path::new("/work"));
        assert_eq!(p, Some(PathBuf::from("/repo/a.py")));
    }

    #[test]
    fn view_and_undo_are_ignored() {
        for cmd in ["view", "undo_edit"] {
            let a = args(json!({"command": cmd, "path": "/repo/a.rs"}));
            assert_eq!(
                edited_path_from_tool_call("developer__text_editor", &a, Path::new("/work")),
                None,
                "command {cmd} should not trigger diagnostics"
            );
        }
    }

    #[test]
    fn non_text_editor_tool_is_ignored() {
        let a = args(json!({"command": "write", "path": "/repo/a.rs"}));
        assert_eq!(
            edited_path_from_tool_call("developer__shell", &a, Path::new("/work")),
            None
        );
    }

    #[test]
    fn empty_or_missing_path_is_ignored() {
        let a = args(json!({"command": "write", "path": "   "}));
        assert_eq!(
            edited_path_from_tool_call("text_editor", &a, Path::new("/work")),
            None
        );
        let a = args(json!({"command": "write"}));
        assert_eq!(
            edited_path_from_tool_call("text_editor", &a, Path::new("/work")),
            None
        );
    }

    #[test]
    fn disabled_config_is_inert() {
        let c = PostEditDiagnosticsConfig::disabled();
        assert!(!c.is_active());
        // Default is now OFF (signed-off: new capabilities ship default-off).
        assert!(!PostEditDiagnosticsConfig::default().is_active());
        // Zero reflections disables even when explicitly enabled.
        assert!(!PostEditDiagnosticsConfig {
            enabled: true,
            max_reflections: 0
        }
        .is_active());
    }

    #[test]
    fn reflection_policy_injects_resets_and_caps() {
        // Under the cap with a dirty batch -> inject and advance.
        assert_eq!(
            next_reflection(true, 0, 3),
            ReflectionOutcome::Inject { next: 1 }
        );
        assert_eq!(
            next_reflection(true, 2, 3),
            ReflectionOutcome::Inject { next: 3 }
        );
        // A clean batch resets, regardless of the running count.
        assert_eq!(next_reflection(false, 2, 3), ReflectionOutcome::Reset);
        // At/over the cap with a still-broken file -> capped, never wedged.
        assert_eq!(next_reflection(true, 3, 3), ReflectionOutcome::Capped);
        assert_eq!(next_reflection(true, 9, 3), ReflectionOutcome::Capped);
        // A zero cap never injects.
        assert_eq!(next_reflection(true, 0, 0), ReflectionOutcome::Capped);
    }

    #[test]
    fn framed_context_names_files_and_lines() {
        let framed = frame_post_edit_diagnostics(&[FileDiagnostics {
            path: "src/main.rs".to_string(),
            lines: vec!["line 3:5: missing `}`".to_string()],
        }]);
        assert!(framed.contains("<post-edit-diagnostics>"));
        assert!(framed.contains("`src/main.rs`"));
        assert!(framed.contains("line 3:5: missing `}`"));
        assert!(framed.contains("false positive"));
    }
}
