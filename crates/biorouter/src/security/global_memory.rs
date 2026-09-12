//! Consent gate for the **machine-wide (global) memory store** (issue #63).
//!
//! # The hole this closes
//!
//! Issue #58 stopped global memory *bodies* being injected into every session's
//! system prompt: the prompt now carries only the category index, and reading a
//! category takes an explicit `retrieve_memories(…, is_global=true)` call. That
//! changed the delivery but not the reachability. Nothing gated the tool call,
//! and in [`BioRouterMode::Auto`] the permission inspector approves every tool
//! deterministically — so a session could still read every global memory any
//! other session ever wrote, with no prompt and nothing shown to the user.
//! `category="*"` returned the entire store in one call.
//!
//! The write side was never gated at all. An in-process MCP server has no
//! channel to the user, so `remember_memory` could only *name* the scope in its
//! result text; the decision to write machine-wide was the model's alone.
//!
//! # The mechanism
//!
//! An **argument-aware** [`ToolInspector`], modelled on
//! [`crate::security::sensitive_ops`]: it reads the memory tools' `is_global`
//! and `category` arguments, not just the tool name, and returns a verdict the
//! escalation-only merge in [`crate::tool_inspection`] applies *over* whatever
//! the permission inspector decided. That is what makes it survive Auto mode —
//! and a user `AlwaysAllow`, and a `SmartApprove` read-only grade — because the
//! merge can raise a verdict but never lower one.
//!
//! Unlike `SensitiveOpsInspector` this gate is **not** inert outside Auto. Auto
//! is merely the loudest case. `SmartApprove` can grade `retrieve_memories` as
//! read-only from its annotations and auto-approve it, and an `AlwaysAllow` the
//! user granted for project-local recall silently covers a machine-wide read
//! too — both are the same undisclosed cross-session read. Only `Chat` is
//! skipped, and only because it dispatches no tools at all.
//!
//! Two properties the rest of the pipeline owes this verdict, each the subject
//! of its own regression suite because each was once missing:
//!
//! * **The ask cannot *lower* a denial.** The merge is a strict
//!   `Deny > RequireApproval > Allow` lattice
//!   ([`crate::tool_inspection::apply_inspection_results_to_permissions`]), so a
//!   call already refused — by a user `NeverAllow`, a managed policy, the
//!   catastrophic-command block — is not also queued for a card. It briefly was,
//!   and since "Allow once" *dispatches* the tool, that card would have run
//!   something already refused.
//! * **No automation may answer the card.** The gate's name is in
//!   [`crate::tool_inspection::NON_DELEGABLE_APPROVAL_INSPECTORS`], so a
//!   `PermissionRequest` hook returning `allow` is dropped and the user is asked
//!   anyway. A hook is one more automated grant, and it runs downstream of the
//!   merge where the lattice can no longer defend the verdict.
//!
//! # What it decides, and why the two verdicts differ
//!
//! * A global read/write/delete naming **one category** →
//!   [`InspectionAction::RequireApproval`]. The user is told which category, in
//!   which direction, and approves or denies it. The feature keeps working;
//!   consent is the point.
//! * `retrieve_memories(category="*", is_global=true)` — the whole-store read —
//!   → [`InspectionAction::Deny`], with a reason that names the itemised call to
//!   use instead.
//!
//! Refusing the bulk *read* while merely asking about the bulk *delete*
//! (`remove_memory_category(category="*", is_global=true)`) is deliberate. The
//! reason is **data minimisation and functional necessity**, not that consent to
//! the bulk read would be impossible to obtain — an earlier draft argued the
//! latter and it does not hold. A card can describe an unseen corpus well enough
//! to decide about ("every global memory on this computer"), a user can
//! knowingly accept that in either direction, and since this branch shipped the
//! inventory in Settings → Chat → Memory the corpus is not even unseen. So:
//!
//! 1. **The disclosure must be no larger than the task.** An answer needs the
//!    memories bearing on the question, and `"*"` hands over every global memory
//!    every other session on the machine ever wrote in order to produce it. The
//!    itemised call discloses what is needed and nothing else, which is the same
//!    reduction issue #58 made when it took the *bodies* out of the system
//!    prompt. Refusing the wide shape is how that reduction is kept: an
//!    always-available bulk read is the one the model reaches for first.
//! 2. **The bulk read is never necessary; the bulk delete has no substitute.**
//!    The system prompt carries every global category name, so a named read is
//!    always available and `"*"` buys exactly one round trip. There is no
//!    equivalent for "forget everything": expressing it as N per-category
//!    deletions is worse for the user, not better — N cards for one intention —
//!    and it cannot reach a category they do not know about. An operation with a
//!    cheap, strictly narrower substitute is refused; one without a substitute
//!    is put to the user.
//! 3. **The two operations leave different things behind.** A disclosure cannot
//!    be withdrawn and it travels — into this transcript, to the provider, into
//!    the context of a conversation that had no business with it. A deletion is
//!    destructive but confined to the user's own machine and is the thing the
//!    user asked for; the card states its blast radius, and the inventory shows
//!    what would be lost beforehand.
//! 4. **It is not a refusal of the feature.** Every byte remains reachable —
//!    one approved category at a time — which is exactly the bar issue #58 set
//!    and the reason the audit would not accept a blanket server-side refusal.
//!
//! # Boundary: the routes an inspector never sees
//!
//! Two production routes dispatch tool calls without going through the agent
//! loop, so no [`ToolInspector`] — this gate included — ever sees them: the
//! calls a script makes from inside `execute_code`, and `POST /agent/call_tool`.
//! A third has no agent at all: `biorouter mcp memory` serves the memory server
//! straight over stdio to any MCP client.
//!
//! The first version of this gate answered the `execute_code` case by **scanning
//! the script text** for an embedded memory call. That was the wrong shape and
//! the #63 review broke it in one line: `const flag = true;
//! retrieve_memories({category:"clinical", is_global:flag})` has no truthy
//! literal after `is_global`, so the scan sees nothing. Source text can always
//! be out-computed.
//!
//! So the decision moved to where there is nothing left to compute:
//!
//! * [`uninspected_boundary_refusal`] is applied at each of the two dispatch
//!   boundaries, reading the *dispatched* tool name and the *evaluated*
//!   arguments. Every global read, write and delete is refused there, and the
//!   model is told to make an ordinary itemised memory call instead — which is
//!   gated, and works.
//! * The memory server itself refuses global operations unless it was built by
//!   the one constructor that states a consent path
//!   (`MemoryServer::behind_consent_gate`), which closes the standalone case
//!   and makes consent a property of the store rather than of one caller.
//!
//! The script scan is **kept**, but it is no longer load-bearing: it is now a
//! warning — deliberately over-inclusive, and harmless when it is wrong because
//! the worst outcome is one approval card — and the card says plainly that the
//! script's memory calls will be refused either way.
//!
//! # Naming the store by path, and merely naming it
//!
//! [`references_global_store`] refuses any tool — not just the four memory ones
//! — that points at the store's path, because the store is a directory of text
//! files and `cat <store>/clinical.txt` is the disclosure the card exists for.
//!
//! It reads only the arguments that carry a *place*: a path, a directory, a
//! shell command line, a script body ([`ArgumentRole`]). Applied to every string
//! in the payload — as it was when #63 first landed — it also refused
//! `text_editor(path="docs/foo.md", file_text="…`~/.config/biorouter/memory`…")`,
//! because [`names_path`] ends a match on a backtick or a space and that is how
//! prose spells a path. The result was an agent that could not document the
//! memory feature, edit this repository's own
//! `docs/extensions/built-in/memory.md`, or commit either.
//!
//! Being unable to *say* where the store is protects nothing: the text is
//! written to the file named beside it, and the model composed the text. What
//! protects the store is the storage boundary in `biorouter-mcp` (see
//! `crates/biorouter-mcp/tests/global_memory_file_barrier.rs`), which resolves
//! the path and refuses the place. This check is the layer above it, and its
//! job is to catch the shell — which resolves no path — naming the store
//! literally.

use serde_json::{Map, Value};

use crate::config::BioRouterMode;
use crate::conversation::message::{Message, ToolRequest};
use crate::security::policy::command::{redirect_targets, ParsedCommand};
use crate::tool_inspection::{InspectionAction, InspectionResult, ToolInspector};
use anyhow::Result;
use async_trait::async_trait;
use uuid::Uuid;

/// This inspector's name, as it appears on an [`InspectionResult`]. Also the key
/// [`crate::agents::Agent::handle_denied_tools`] matches on so a refusal reaches
/// the model as its real reason instead of "the user declined".
pub const GLOBAL_MEMORY_INSPECTOR_NAME: &str = "global_memory";

/// The verdict for one tool call against the global memory store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlobalMemoryGate {
    /// Route to the standard approval flow, showing this message.
    Ask(String),
    /// Refuse outright, telling the model this reason (and how to proceed).
    Refuse(String),
}

/// The `biorouter-mcp` memory server's four tools, named as the *tool* names
/// they are declared with. The `<extension>__` prefix the extension manager adds
/// is configuration (an operator can mount the server under any name), so it is
/// deliberately not part of the match.
const RETRIEVE_MEMORIES: &str = "retrieve_memories";
const REMEMBER_MEMORY: &str = "remember_memory";
const REMOVE_MEMORY_CATEGORY: &str = "remove_memory_category";
const REMOVE_SPECIFIC_MEMORY: &str = "remove_specific_memory";

const MEMORY_TOOL_NAMES: &[&str] = &[
    RETRIEVE_MEMORIES,
    REMEMBER_MEMORY,
    REMOVE_MEMORY_CATEGORY,
    REMOVE_SPECIFIC_MEMORY,
];

/// The documented "every category" sentinel, matched exactly as the memory
/// server dispatches it (`params.category == "*"`). Issue #73 keeps `*` a legal
/// plain category name as well, so a padded `" * "` is an ordinary category and
/// must not be reported to the user as a whole-store operation.
const ALL_CATEGORIES: &str = "*";

/// Which memory tool this is, from the tool name as the model called it.
fn memory_tool(tool_name: &str) -> Option<&'static str> {
    // `a__b` → `b`; a bare `b` → `b`.
    let base = tool_name
        .rsplit("__")
        .next()
        .unwrap_or(tool_name)
        .to_ascii_lowercase();
    MEMORY_TOOL_NAMES.iter().copied().find(|n| *n == base)
}

/// Whether a JSON value reads as `true`.
///
/// `is_global` is declared `bool`, but a model that sends `"true"` would
/// otherwise slip past a strict `as_bool()` and reach the store ungated — and
/// `serde` would still accept it in some shapes. Leniency here only ever *adds*
/// a consent prompt. An absent or unrecognised flag is **not** treated as
/// global: local is the tool's documented default, so failing that way turns a
/// malformed call into a local one, never into a silent machine-wide one.
fn reads_as_true(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::String(s) => matches!(s.trim().to_ascii_lowercase().as_str(), "true" | "yes" | "1"),
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        _ => false,
    }
}

fn targets_global_store(args: &Map<String, Value>) -> bool {
    args.get("is_global").is_some_and(reads_as_true)
}

fn category_of(args: &Map<String, Value>) -> Option<&str> {
    args.get("category").and_then(Value::as_str)
}

/// The body `remove_specific_memory` is about to destroy, so the card can name
/// it. Not a disclosure: the model supplied this text, so the user is being
/// shown what the call already contains.
fn memory_content_of(args: &Map<String, Value>) -> Option<&str> {
    args.get("memory_content").and_then(Value::as_str)
}

/// Keep an approval card readable when a memory runs to paragraphs. The user is
/// identifying which memory, not re-reading it.
fn elided(text: &str) -> String {
    const MAX: usize = 240;
    if text.chars().count() <= MAX {
        return text.to_string();
    }
    let head: String = text.chars().take(MAX).collect();
    format!("{}…", head.trim_end())
}

/// True for the code-execution tool, whose JS body carries the real (inner) tool
/// calls. Those are dispatched straight through the extension manager and never
/// reach an agent-layer inspector, so the body itself has to be inspected.
fn is_execute_code(tool_name: &str) -> bool {
    tool_name.to_ascii_lowercase().contains("execute_code")
}

/// Does this script set `is_global` to something truthy anywhere?
///
/// Deliberately crude and deliberately over-inclusive: the cost of a false
/// positive is one approval prompt on a script that also mentions memory, and
/// the cost of a false negative is the ungated cross-session read this whole
/// module exists to stop.
fn sets_global_true(lowercased: &str) -> bool {
    // `split` + `skip(1)` yields the text following each `is_global` occurrence,
    // without indexing into the string by byte offset.
    lowercased.split("is_global").skip(1).any(|after| {
        let value = after.trim_start_matches(|c: char| {
            c.is_whitespace() || matches!(c, ':' | '=' | '"' | '\'' | '`')
        });
        value.starts_with("true") || value.starts_with("yes") || value.starts_with('1')
    })
}

/// A global-memory call embedded in an `execute_code` body.
fn code_touches_global_memory(code: &str) -> bool {
    let lowercased = code.to_ascii_lowercase();
    MEMORY_TOOL_NAMES
        .iter()
        .any(|name| lowercased.contains(name))
        && sets_global_true(&lowercased)
}

/// Every spelling of the machine-wide store's path a tool argument might carry:
/// the absolute path, and — when the store is under the user's home — the `~`,
/// `$HOME` and `${HOME}` abbreviations a shell command normally uses.
fn global_store_path_forms() -> Vec<String> {
    let store = biorouter_mcp::global_memory_dir();
    let mut forms = vec![store.to_string_lossy().into_owned()];
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        if let Ok(tail) = store.strip_prefix(std::path::Path::new(&home)) {
            let tail = tail.to_string_lossy();
            forms.push(format!("~/{tail}"));
            forms.push(format!("$HOME/{tail}"));
            forms.push(format!("${{HOME}}/{tail}"));
        }
    }
    forms
}

/// Does `text` name `store` *as a path*, rather than merely start with its
/// characters?
///
/// The distinction is the whole difference between a fix and an outage:
/// `<store>-archive.txt` and `<store>notes` are ordinary files a plain
/// `contains` would deny, while `<store>/clinical.txt`, a bare `<store>`, and
/// `"<store>"` inside a quoted command are the store. So the match has to end on
/// a path boundary.
fn names_path(text: &str, store: &str) -> bool {
    text.match_indices(store).any(|(at, matched)| {
        text.get(at + matched.len()..)
            .and_then(|after| after.chars().next())
            // Nothing after the match is the bare store path, which counts.
            .is_none_or(|c| {
                c == '/' || c == '\\' || c.is_whitespace() || "\"'`)];,;:&|".contains(c)
            })
    })
}

/// What an argument's value *is*, which decides how a mention of the store's
/// path inside it should be read.
///
/// The distinction is the whole of the #63 second-round fix. The check used to
/// walk **every string in the payload**, and `names_path` ends a match on a
/// backtick, a quote or whitespace — which is precisely how prose spells a
/// path. So `text_editor(command="write", path="docs/foo.md", file_text="Global
/// memories live in `~/.config/biorouter/memory`…")` was refused: an agent
/// could not document the memory feature, edit this repository's own
/// `docs/extensions/built-in/memory.md`, or write the release notes for the
/// gate itself.
///
/// Nothing in that call goes near the store. `file_text` is written to the
/// `path` beside it; the path is a *quotation*. So the check now asks which
/// argument carries the mention, and only refuses the ones where naming the
/// store means opening it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArgumentRole {
    /// A filesystem path the tool will resolve and open.
    Path,
    /// A shell command line, read as the shell reads it (see
    /// [`shell_command_names_store`]).
    ShellCommand,
    /// Executable source text — a script body, whose own literal references are
    /// scanned whole.
    Script,
}

/// Argument keys whose value is a **directory**, on top of the shared
/// [`crate::security::is_path_argument_key`] list.
///
/// The store is a directory, so a tool pointed at it by a directory-valued
/// argument reaches it just as surely as one given a file path:
/// `shell(working_directory=<store>)` runs `ls` inside it, and
/// `create_app(capabilities.files.entries[].local_dir=<store>)` mounts it into
/// an app's workspace. Enumerated from the declared schemas —
/// `ShellParams::working_directory`, `files_server::ListParams::dir`,
/// `agent_drafter` `export_app.target_dir` and `WorkspaceEntry::local_dir` —
/// not guessed.
///
/// They are kept here rather than added to the shared list because widening
/// that list also widens what [`crate::security::sensitive_ops`] escalates,
/// which is a separate decision with its own regression suite.
const DIRECTORY_ARG_KEYS: &[&str] = &["dir", "directory", "working_directory", "local_dir"];

/// Argument keys whose value is executable source text.
///
/// `execute_code.code` and `compute_python.code` (JS / Python bodies), and
/// `automation_script.script` / `computer_control.script` (shell, batch,
/// PowerShell, AppleScript).
const SCRIPT_ARG_KEYS: &[&str] = &["code", "script"];

/// How the value under `key` should be read, or `None` if a path inside it is
/// not a reference to a place.
fn argument_role(key: &str) -> Option<ArgumentRole> {
    let lowered = key.to_ascii_lowercase();
    if crate::security::is_path_argument_key(&lowered)
        || DIRECTORY_ARG_KEYS.contains(&lowered.as_str())
    {
        return Some(ArgumentRole::Path);
    }
    if crate::security::is_command_argument_key(&lowered) {
        return Some(ArgumentRole::ShellCommand);
    }
    if SCRIPT_ARG_KEYS.contains(&lowered.as_str()) {
        return Some(ArgumentRole::Script);
    }
    None
}

/// Does this shell command line name the store *as a path operand*?
///
/// A raw substring search cannot answer this, because the same characters mean
/// two different things in a command line:
///
/// ```text
/// cat ~/.config/biorouter/memory/clinical.txt          <- reads the store
/// git commit -m "docs: ~/.config/biorouter/memory is shared"   <- talks about it
/// ```
///
/// So the command is handed to [`ParsedCommand`], the policy engine's tokenizer,
/// and each resulting argv token is tested. Quoting is what separates the two
/// cases: `shlex` gives the commit message back as **one token** ("docs: … is
/// shared"), which is not a path, while the `cat` operand is a token that *is*
/// the store. Reusing the parser rather than splitting on whitespace also buys
/// the pipeline/`;` split, redirect targets, and — because it recurses through
/// `sh -c` — a store reference nested inside `bash -c "cat <store>/x"`.
///
/// A token *starts* with the store and continues with a path separator, or is
/// the store exactly. Unlike [`names_path`] a whitespace boundary does not
/// count here: whitespace inside a token means the token is a quoted sentence,
/// which is the case this function exists to let through.
///
/// Not caught: `--dir=<store>`, and any command that computes the path rather
/// than writing it. That is the same limit the module docs already state — this
/// check is a literal-reference warning, and the store's actual guarantee is the
/// storage boundary in `biorouter-mcp`, which resolves paths and cannot be
/// out-computed.
fn shell_command_names_store(command: &str, forms: &[String]) -> bool {
    let token_is_store = |token: &str| {
        forms.iter().any(|form| {
            token.strip_prefix(form.as_str()).is_some_and(|rest| {
                rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\')
            })
        })
    };
    let parsed = ParsedCommand::parse(command, std::path::Path::new("."));
    parsed
        .segments
        .iter()
        .flat_map(|segment| segment.argv.iter())
        .any(|token| token_is_store(token))
        // `> <store>/x` is a write target and not an argv token.
        || redirect_targets(command).iter().any(|t| token_is_store(t))
}

/// Does `args` name the machine-wide memory store *in an argument that would
/// open it*?
///
/// Walks nested objects and arrays, because a path arrives as
/// `{"opts": {"paths": ["…"]}}` or `{"params": {"image_path": "…"}}` as readily
/// as a top-level `path`; the walk descends through keys with no role of their
/// own, looking for one that has.
///
/// The alternative shape considered and rejected was to keep scanning every
/// string and merely downgrade a prose match from a refusal to an approval
/// card. That still breaks the case: in Auto mode a card stops an autonomous
/// run, so every documentation edit that quotes the path would block on a human
/// — and the card would be asking consent for a disclosure that is not
/// happening, since the model wrote the text it is being asked about. A prose
/// mention is not a memory operation and should not be treated as one.
fn references_global_store(args: &Map<String, Value>) -> bool {
    fn in_map(map: &Map<String, Value>, forms: &[String]) -> bool {
        map.iter().any(|(key, value)| match argument_role(key) {
            Some(role) => in_role(value, forms, role),
            // Not a path-bearing key itself; a path-bearing one may be nested
            // under it.
            None => descend(value, forms),
        })
    }

    fn descend(value: &Value, forms: &[String]) -> bool {
        match value {
            Value::Object(map) => in_map(map, forms),
            Value::Array(items) => items.iter().any(|item| descend(item, forms)),
            _ => false,
        }
    }

    fn in_role(value: &Value, forms: &[String], role: ArgumentRole) -> bool {
        match value {
            Value::String(text) => match role {
                ArgumentRole::Path | ArgumentRole::Script => {
                    forms.iter().any(|form| names_path(text, form))
                }
                ArgumentRole::ShellCommand => shell_command_names_store(text, forms),
            },
            // `paths: [...]`, and the odd tool that nests a path one level down.
            Value::Array(items) => items.iter().any(|item| in_role(item, forms, role)),
            Value::Object(map) => map.values().any(|v| in_role(v, forms, role)),
            _ => false,
        }
    }

    let forms = global_store_path_forms();
    in_map(args, &forms)
}

/// What the model is told when a tool argument points at the store's path.
///
/// Naming the itemised call that *does* work is the load-bearing half: a
/// refusal that only says no gets retried, or reported to the user as a broken
/// feature.
fn named_by_path_refusal(tool_name: &str) -> String {
    format!(
        "Refused: this call names Biorouter's machine-wide memory store by path. That store \
         is shared by every Biorouter session on this computer, and reading, changing or \
         deleting it has to be shown to the user and approved by category, which a file \
         path cannot be. Use the memory tools instead: \
         retrieve_memories(category=\"<name>\", is_global=true) to read a category, \
         remember_memory(...) to add to one, remove_memory_category / \
         remove_specific_memory to delete. Each is put to the user by name. To browse or \
         prune the store themselves the user can open Settings → Chat → Memory. \
         Project-local memory (.biorouter/memory) is not affected.{}",
        if is_execute_code(tool_name) {
            " Note this call is a script: make the memory call directly, outside execute_code."
        } else {
            ""
        }
    )
}

/// The card shown for an `execute_code` body that looks like it wants global
/// memory. It asks about the *rest* of the script, because the memory calls
/// themselves are refused at the dispatch boundary whatever the user answers.
fn script_touches_memory_card() -> String {
    "🔒 This script looks like it wants cross-session memory.\n\
     It appears to read or write the global memory store, the machine-wide store \
     shared by every Biorouter session on this computer, in every project. Tool \
     calls made from inside a script are not itemised, so there is nothing here for \
     you to approve one by one, and Biorouter therefore refuses global memory \
     operations attempted from inside a script whatever you decide now.\n\
     So this asks about the rest of what the script does. Approve it to run it (its \
     global memory calls will fail with an explanation) or deny it and ask for the \
     memory calls to be made directly, where each one is shown to you by category."
        .to_string()
}

/// Classify a tool call against the global memory store.
///
/// `None` means "this call does not touch the machine-wide store" — a local
/// memory call, or any other tool at all.
pub fn global_memory_gate(tool_name: &str, args: &Map<String, Value>) -> Option<GlobalMemoryGate> {
    // Issue #63 review, finding 2. The gate below matches four tool *names*, but
    // the store is a directory of text files: `cat <store>/clinical.txt` is the
    // same disclosure the card exists for, and `rm -rf <store>` the same
    // destruction, with a tool this gate does not recognise.
    //
    // The developer and computer-controller servers now refuse the store at the
    // filesystem boundary, which is the unforgeable half. The shell resolves no
    // path, so its *literal* references are caught here — refused rather than
    // asked, because approving "run this shell command" is not consent to
    // disclose a memory category, and there is a call that is.
    //
    // Checked before everything else so it covers every tool, `execute_code`
    // bodies included. It does **not** close the general hole: an obfuscated or
    // computed shell command still reads any file on the machine. That is the
    // filesystem barrier of issue #56, deliberately not built here.
    //
    // What it checks is the arguments that *carry a place* — see
    // [`ArgumentRole`]. Applied to the whole payload it refused any call that
    // merely quoted the path, which took documentation, release notes and
    // commit messages about this very feature with it.
    if references_global_store(args) {
        return Some(GlobalMemoryGate::Refuse(named_by_path_refusal(tool_name)));
    }

    if is_execute_code(tool_name) {
        let code = args.get("code").and_then(Value::as_str)?;
        return code_touches_global_memory(code)
            .then(|| GlobalMemoryGate::Ask(script_touches_memory_card()));
    }

    let tool = memory_tool(tool_name)?;
    if !targets_global_store(args) {
        // Project-local memory lives under the working directory the user
        // opened, so it crosses no session boundary and is left alone.
        return None;
    }
    let category = category_of(args)?;
    let all_categories = category == ALL_CATEGORIES;

    Some(match (tool, all_categories) {
        // The whole-store read: refused, never asked about. See the module docs
        // — the disclosure must be no larger than the task, and this shape has a
        // strictly narrower substitute that is always available.
        (RETRIEVE_MEMORIES, true) => GlobalMemoryGate::Refuse(
            "Refused: retrieve_memories(category=\"*\", is_global=true) would disclose the entire \
             machine-wide memory store, every global memory written by every other session on \
             this computer, in one call, to answer a question that needs some of it. Read one \
             category at a time instead: retrieve_memories(category=\"<name>\", is_global=true), \
             which asks the user about that category by name. The global category names are \
             listed in your system prompt, so nothing is out of reach. Local bulk retrieval \
             (is_global=false) is unaffected."
                .to_string(),
        ),
        (RETRIEVE_MEMORIES, false) => GlobalMemoryGate::Ask(format!(
            "🔒 Cross-session memory read.\n\
             This conversation is asking to read the global memory category \"{category}\": the \
             machine-wide store that every Biorouter session on this computer shares, including \
             sessions in other projects. Its contents were written by other conversations and are \
             not otherwise part of this one.\n\
             Approve it to disclose that category here, or deny it. To read \"{category}\" \
             yourself first, or delete what is in it, open Settings → Chat → Memory. \
             Project-local memories (.biorouter/memory) are unaffected and never prompt."
        )),
        (REMEMBER_MEMORY, _) => GlobalMemoryGate::Ask(format!(
            "🔒 Cross-session memory write.\n\
             This conversation is asking to save a memory to the global category \"{category}\", \
             the machine-wide store, readable from now on by every Biorouter session on this \
             computer, in every project.\n\
             Approve it to let this note follow you across projects, or deny it and ask for a \
             project-local memory instead."
        )),
        // Clearing the whole store is asked about rather than refused: unlike
        // the bulk read it has no narrower substitute, and the inventory shows
        // the user what would be lost before they answer.
        //
        // The three deletions below destroy three different amounts and each
        // card says which. They shared one "delete from category" message, which
        // told a user nothing about whether they were losing one note or the
        // whole category, and consenting to an irreversible loss you cannot size
        // is not consent (#63 review, finding 6).
        (REMOVE_MEMORY_CATEGORY, true) => GlobalMemoryGate::Ask(
            "🔒 Deletes every global memory.\n\
             This conversation is asking to clear the entire machine-wide memory store: every \
             global category any session on this computer ever saved, and everything in them. \
             Biorouter keeps no copy: this cannot be undone.\n\
             Approve it only if you asked for your global memories to be wiped. To see what \
             would be lost, or to delete categories one at a time instead, open \
             Settings → Chat → Memory. Project-local memories (.biorouter/memory) are \
             unaffected."
                .to_string(),
        ),
        (REMOVE_MEMORY_CATEGORY, false) => GlobalMemoryGate::Ask(format!(
            "🔒 Deletes a whole global memory category.\n\
             This conversation is asking to delete every memory in the global category \
             \"{category}\", and the category itself, from the machine-wide store shared by \
             every Biorouter session on this computer, so it goes for every project. Biorouter \
             keeps no copy: this cannot be undone.\n\
             To see what is in \"{category}\" before you decide, or to delete single memories \
             from it instead, open Settings → Chat → Memory."
        )),
        (REMOVE_SPECIFIC_MEMORY, _) => GlobalMemoryGate::Ask(format!(
            "🔒 Deletes one global memory.\n\
             This conversation is asking to delete one memory from the global category \
             \"{category}\", the machine-wide store shared by every Biorouter session on this \
             computer, so it goes for every project. The rest of \"{category}\" is kept. \
             Biorouter keeps no copy of what is removed: this cannot be undone.\n\
             {quoted}\n\
             To see the whole category first, open Settings → Chat → Memory.",
            quoted = match memory_content_of(args) {
                // Quoting it is the only way the user can tell *which* memory is
                // about to go; the model already has this text, so showing it
                // discloses nothing the card does not already presuppose.
                Some(body) => format!("The memory: \"{}\"", elided(body)),
                None => "The call did not say which memory.".to_string(),
            }
        )),
        // `memory_tool` returns one of the four constants above; a new tool
        // added to the server without a rule here fails closed.
        _ => GlobalMemoryGate::Ask(format!(
            "🔒 Cross-session memory access.\n\
             This conversation is asking to use the global memory category \"{category}\": the \
             machine-wide store shared by every Biorouter session on this computer, in every \
             project.\n\
             Approve it, or deny it."
        )),
    })
}

/// The two dispatch boundaries no [`ToolInspector`] sees, this gate included
/// (issue #63 review, finding 3).
///
/// ⚠ **Defined in [`crate::security`], not here, and re-exported for the call
/// sites that already spell it `global_memory::UninspectedBoundary`.** It stopped
/// being this module's private vocabulary the moment a second gate
/// ([`crate::security::session_store`]) had to close the same two doors — see the
/// type's own docs for why one list rather than two.
pub use crate::security::UninspectedBoundary;

impl UninspectedBoundary {
    /// How the refusal names where the call came from, and what to do instead.
    fn remedy(self) -> &'static str {
        match self {
            Self::ExecuteCodeScript => {
                "Tool calls made from inside a script are not shown to the user one by one, so \
                 there is nothing for them to approve. Make the memory call directly, outside \
                 execute_code, with retrieve_memories / remember_memory / remove_memory_category / \
                 remove_specific_memory and is_global=true, and the user will be shown that \
                 exact operation and asked to approve it."
            }
            Self::AgentCallToolRoute => {
                "This route dispatches a tool without the agent loop, so nothing here can put \
                 the operation to the user. Issue the memory call in a conversation instead, \
                 where it is inspected and approved."
            }
        }
    }
}

/// Is this a memory-tool call against the machine-wide store?
///
/// Unlike [`code_touches_global_memory`], which reads *source text* and can
/// therefore always be out-computed, this reads the dispatched call: the real
/// tool name and the real, already-evaluated arguments. There is nothing left to
/// hide behind — `is_global: flag` and `is_global: true` arrive here identically.
fn is_global_store_call(tool_name: &str, args: &Map<String, Value>) -> bool {
    memory_tool(tool_name).is_some() && targets_global_store(args)
}

/// The refusal a dispatch boundary that cannot ask the user must return for a
/// machine-wide memory operation, or `None` if the call is not one.
///
/// This is the answer to "the static `execute_code` scan can be evaded". It is
/// not a better scan; it is the check moved to where evasion is impossible.
/// Every global read, write and delete is refused at these boundaries — a
/// boundary that cannot obtain consent does not act without it — and the model
/// is told to make an ordinary itemised memory call, which *is* gated and does
/// work. Local memory is untouched.
pub fn uninspected_boundary_refusal(
    tool_name: &str,
    args: Option<&Map<String, Value>>,
    boundary: UninspectedBoundary,
) -> Option<String> {
    // Two ways to reach the store, both closed here: as a memory tool call
    // against it, and — since a boundary sees no inspector either — as any tool
    // at all naming its path (finding 2's shell case, arriving through a script
    // rather than through the agent loop).
    let args = args?;
    if !is_global_store_call(tool_name, args) && !references_global_store(args) {
        return None;
    }
    tracing::warn!(
        counter.biorouter.global_memory_uninspected_refused = 1,
        tool_name = %tool_name,
        boundary = ?boundary,
        "Refused a machine-wide memory operation at a boundary that cannot ask the user"
    );
    Some(format!(
        "Refused: this call would read, write or delete the global memory store, the \
         machine-wide store shared by every Biorouter session on this computer, in every \
         project, and every such operation has to be shown to the user and approved first. \
         {}",
        boundary.remedy()
    ))
}

/// Inspector that routes global-memory access through the user, in every mode
/// that runs tools. See the module docs for the policy.
pub struct GlobalMemoryInspector;

#[async_trait]
impl ToolInspector for GlobalMemoryInspector {
    fn name(&self) -> &'static str {
        GLOBAL_MEMORY_INSPECTOR_NAME
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn inspect(
        &self,
        tool_requests: &[ToolRequest],
        _messages: &[Message],
        biorouter_mode: BioRouterMode,
        _session: &crate::session::Session,
    ) -> Result<Vec<InspectionResult>> {
        // Chat dispatches no tools; the agent splices a canned response.
        if biorouter_mode == BioRouterMode::Chat {
            return Ok(vec![]);
        }

        let mut results = Vec::new();
        for request in tool_requests {
            let Ok(tool_call) = &request.tool_call else {
                continue;
            };
            let Some(args) = tool_call.arguments.as_ref() else {
                continue;
            };
            let Some(gate) = global_memory_gate(&tool_call.name, args) else {
                continue;
            };
            let (action, reason) = match gate {
                GlobalMemoryGate::Ask(message) => (
                    InspectionAction::RequireApproval(Some(message.clone())),
                    message,
                ),
                GlobalMemoryGate::Refuse(reason) => (InspectionAction::Deny, reason),
            };
            tracing::warn!(
                counter.biorouter.global_memory_gated = 1,
                tool_name = %tool_call.name,
                tool_request_id = %request.id,
                denied = matches!(action, InspectionAction::Deny),
                "Global memory access routed through the user"
            );
            results.push(InspectionResult {
                tool_request_id: request.id.clone(),
                action,
                reason,
                confidence: 1.0,
                inspector_name: self.name().to_string(),
                finding_id: Some(format!("GMEM-{}", Uuid::new_v4().simple())),
            });
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::CallToolRequestParams;
    use serde_json::json;

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    // --- classification: the reads ----------------------------------------

    /// The issue's exact demonstration: `category="*"` + `is_global=true` hands
    /// back the entire machine-wide store in one call. It is refused, not asked
    /// about — see the module docs: the disclosure must be no larger than the
    /// task, and the itemised read is always available.
    #[test]
    fn the_whole_store_read_is_refused() {
        let gate = global_memory_gate(
            "memory__retrieve_memories",
            &args(json!({"category": "*", "is_global": true})),
        );
        let Some(GlobalMemoryGate::Refuse(reason)) = gate else {
            panic!("retrieve_memories(\"*\", is_global=true) must be refused, got {gate:?}");
        };
        assert!(
            reason.contains("retrieve_memories") && reason.contains("category"),
            "the refusal must name the itemised call that still works, or the \
             feature is dead: {reason}"
        );
    }

    /// A named global category is the shape the feature is *for*. It is asked
    /// about, never refused, and the card names the category.
    #[test]
    fn a_named_global_read_is_asked_about() {
        let gate = global_memory_gate(
            "memory__retrieve_memories",
            &args(json!({"category": "clinical", "is_global": true})),
        );
        let Some(GlobalMemoryGate::Ask(message)) = gate else {
            panic!("a named global read must ask, got {gate:?}");
        };
        assert!(
            message.contains("clinical"),
            "the approval card must name the category being disclosed: {message}"
        );
    }

    /// Naming the category is only half an answer. The user's actual question
    /// on seeing this card is "what is *in* `clinical`?", and there is now a
    /// place that answers it — so the card has to say where, or the approval is
    /// still over text the user has never seen.
    ///
    /// The two cards that need it are the ones whose subject is *content the
    /// user cannot see*: the read (what am I disclosing?) and the whole-store
    /// wipe (what am I destroying?). The write card carries its own subject —
    /// the model just supplied the text — and a card per call would make the
    /// pointer noise rather than help.
    #[test]
    fn the_cards_about_unseen_content_say_where_to_see_it() {
        for (tool, arguments) in [
            (
                "memory__retrieve_memories",
                json!({"category": "clinical", "is_global": true}),
            ),
            (
                "memory__remove_memory_category",
                json!({"category": "*", "is_global": true}),
            ),
            // The two deletions that name a category: what is in it is the
            // whole question of whether to approve.
            (
                "memory__remove_memory_category",
                json!({"category": "clinical", "is_global": true}),
            ),
            (
                "memory__remove_specific_memory",
                json!({"category": "clinical", "memory_content": "x", "is_global": true}),
            ),
        ] {
            let gate = global_memory_gate(tool, &args(arguments));
            let Some(GlobalMemoryGate::Ask(message)) = gate else {
                panic!("{tool} must ask, got {gate:?}");
            };
            assert!(
                message.contains("Settings")
                    && message.contains("Chat")
                    && message.contains("Memory"),
                "{tool}'s card must point at the surface that lists the store, \
                 by the path the user would actually follow: {message}"
            );
        }
    }

    // --- classification: the writes ---------------------------------------

    /// CLAUDE.md records the write as the gap #58 could not close from inside an
    /// MCP server ("a real confirmation needs the permission path"). This is it.
    #[test]
    fn a_global_write_is_asked_about() {
        let gate = global_memory_gate(
            "memory__remember_memory",
            &args(json!({
                "category": "clinical",
                "data": "cohort 4217 had 12 responders",
                "is_global": true
            })),
        );
        let Some(GlobalMemoryGate::Ask(message)) = gate else {
            panic!("a global write must ask, got {gate:?}");
        };
        assert!(
            message.contains("clinical"),
            "the card must name the category being written: {message}"
        );
        assert!(
            !message.contains("cohort 4217"),
            "the card is a consent prompt, not a second copy of the payload: {message}"
        );
    }

    #[test]
    fn a_global_delete_is_asked_about() {
        for (tool, arguments) in [
            (
                "memory__remove_memory_category",
                json!({"category": "clinical", "is_global": true}),
            ),
            (
                "memory__remove_specific_memory",
                json!({"category": "clinical", "memory_content": "x", "is_global": true}),
            ),
        ] {
            assert!(
                matches!(
                    global_memory_gate(tool, &args(arguments)),
                    Some(GlobalMemoryGate::Ask(_))
                ),
                "{tool} against the global store must ask"
            );
        }
    }

    /// The three deletions destroy three different amounts, irreversibly, and
    /// each card has to say which — a user reading "delete from the global
    /// memory category `clinical`" cannot tell whether they are about to lose
    /// one note or the whole category (#63 review, finding 6).
    #[test]
    fn each_deletion_card_states_its_own_blast_radius() {
        let card = |tool: &str, arguments: Value| match global_memory_gate(tool, &args(arguments)) {
            Some(GlobalMemoryGate::Ask(message)) => message,
            other => panic!("{tool} must ask, got {other:?}"),
        };

        let one_entry = card(
            "memory__remove_specific_memory",
            json!({"category": "clinical", "memory_content": "cohort 4217 responded",
                   "is_global": true}),
        );
        let whole_category = card(
            "memory__remove_memory_category",
            json!({"category": "clinical", "is_global": true}),
        );
        let whole_store = card(
            "memory__remove_memory_category",
            json!({"category": "*", "is_global": true}),
        );

        // Each says how much goes, in words the other two do not use — so the
        // three cards cannot be told apart only by squinting at the arguments.
        assert!(
            one_entry.contains("one memory") && one_entry.contains("The rest of \"clinical\""),
            "the single-entry card must say one memory goes and the rest stays: {one_entry}"
        );
        assert!(
            !one_entry.contains("every memory in"),
            "the single-entry card must not read as a whole-category delete: {one_entry}"
        );
        assert!(
            one_entry.contains("cohort 4217 responded"),
            "the single-entry card must quote the memory being destroyed: it is \
             the only way the user can tell which one: {one_entry}"
        );
        assert!(
            whole_category.contains("every memory in") && whole_category.contains("clinical"),
            "the whole-category card must say the category and everything in it \
             goes: {whole_category}"
        );
        assert!(
            !whole_category.contains("one memory"),
            "the whole-category card must not read as a single-memory delete: {whole_category}"
        );
        assert!(
            whole_store.contains("every global category"),
            "the whole-store card must say every category goes: {whole_store}"
        );

        for (what, message) in [
            ("single entry", &one_entry),
            ("whole category", &whole_category),
            ("whole store", &whole_store),
        ] {
            assert!(
                message.contains("cannot be undone"),
                "the {what} card must say the loss is irreversible: Biorouter \
                 keeps no copy: {message}"
            );
        }
    }

    /// Wiping the whole global store is asked about rather than refused: unlike
    /// the bulk read, it has no narrower substitute that achieves the same thing
    /// — N per-category deletions are N cards for one intention, and cannot
    /// reach a category the user does not know about.
    #[test]
    fn the_whole_store_delete_is_asked_about_not_refused() {
        let gate = global_memory_gate(
            "memory__remove_memory_category",
            &args(json!({"category": "*", "is_global": true})),
        );
        let Some(GlobalMemoryGate::Ask(message)) = gate else {
            panic!("clearing the whole global store must ask, got {gate:?}");
        };
        assert!(
            message.contains("every"),
            "the card has to say the blast radius is every global category: {message}"
        );
    }

    // --- the feature is not disabled --------------------------------------

    /// Local memory is untouched by this gate — including the bulk read, which
    /// crosses no session boundary because `.biorouter/memory` lives under the
    /// working directory the user opened.
    #[test]
    fn local_memory_is_never_gated() {
        for (tool, arguments) in [
            (
                "memory__retrieve_memories",
                json!({"category": "*", "is_global": false}),
            ),
            (
                "memory__retrieve_memories",
                json!({"category": "development", "is_global": false}),
            ),
            (
                "memory__remember_memory",
                json!({"category": "development", "data": "x", "is_global": false}),
            ),
            (
                "memory__remove_memory_category",
                json!({"category": "*", "is_global": false}),
            ),
        ] {
            assert!(
                global_memory_gate(tool, &args(arguments)).is_none(),
                "{tool} on the local store must not be gated"
            );
        }
    }

    /// Nothing else in the tool surface is touched.
    #[test]
    fn unrelated_tools_are_not_gated() {
        for (tool, arguments) in [
            ("developer__shell", json!({"command": "ls"})),
            (
                "developer__text_editor",
                json!({"command": "write", "path": "/tmp/x"}),
            ),
            // A different extension that happens to carry an `is_global` flag.
            (
                "workflow__save_workflow",
                json!({"name": "x", "is_global": true}),
            ),
        ] {
            assert!(
                global_memory_gate(tool, &args(arguments)).is_none(),
                "{tool} is not a memory tool and must not be gated"
            );
        }
    }

    // --- robustness of the argument read ----------------------------------

    /// The extension prefix is configuration, not a contract: the gate keys off
    /// the tool's own name so a memory server mounted under another name is
    /// still gated.
    #[test]
    fn the_extension_prefix_is_not_load_bearing() {
        for tool in [
            "retrieve_memories",
            "memory__retrieve_memories",
            "personal_memory__retrieve_memories",
        ] {
            assert!(
                global_memory_gate(tool, &args(json!({"category": "c", "is_global": true})))
                    .is_some(),
                "{tool} must be recognised as a memory retrieval"
            );
        }
    }

    /// Models send booleans as strings often enough that a strict `as_bool`
    /// would be a silent bypass. Anything that reads as true counts as global;
    /// a missing flag does not (the tool's own default, and its schema, is
    /// local).
    #[test]
    fn a_stringly_typed_global_flag_still_gates() {
        for flag in [json!(true), json!("true"), json!("True"), json!(1)] {
            assert!(
                global_memory_gate(
                    "memory__retrieve_memories",
                    &args(json!({"category": "c", "is_global": flag}))
                )
                .is_some(),
                "is_global={flag} must gate"
            );
        }
        for flag in [json!(false), json!("false"), json!(0), json!(null)] {
            assert!(
                global_memory_gate(
                    "memory__retrieve_memories",
                    &args(json!({"category": "c", "is_global": flag}))
                )
                .is_none(),
                "is_global={flag} is not a global call"
            );
        }
        assert!(
            global_memory_gate("memory__retrieve_memories", &args(json!({"category": "c"})))
                .is_none(),
            "an absent is_global is a local call"
        );
    }

    /// `"*"` is the sentinel only when it is exactly `"*"` — that is how the
    /// memory server dispatches it (issue #73 keeps it a legal plain name too),
    /// so a padded `" * "` is an ordinary category and must be asked about, not
    /// reported to the user as a whole-store read.
    #[test]
    fn only_the_exact_star_is_the_bulk_sentinel() {
        assert!(
            matches!(
                global_memory_gate(
                    "memory__retrieve_memories",
                    &args(json!({"category": " * ", "is_global": true}))
                ),
                Some(GlobalMemoryGate::Ask(_))
            ),
            "a padded star is a plain category name, not the sentinel"
        );
    }

    // --- the execute_code bypass ------------------------------------------

    /// `execute_code`'s inner tool calls are dispatched straight through the
    /// extension manager and never reach an inspector, so a global read hidden
    /// in the script would otherwise run unseen (the same gap `sensitive_ops`
    /// documents as R2-01).
    #[test]
    fn a_global_read_hidden_in_execute_code_is_escalated() {
        let code = r#"import { retrieve_memories } from "memory";
const all = retrieve_memories({ category: "*", is_global: true });
record_result(all);"#;
        assert!(
            matches!(
                global_memory_gate("code_execution__execute_code", &args(json!({"code": code}))),
                Some(GlobalMemoryGate::Ask(_))
            ),
            "an embedded global memory read must escalate the execute_code call"
        );
    }

    // --- generic tools that name the store's path -------------------------

    /// Every spelling a model might use, built independently of the production
    /// helper so the test does not agree with the code by construction.
    /// Hold `BIOROUTER_PATH_ROOT` still for the duration of a test.
    ///
    /// ⚠ Every test below resolves the store **twice** — once here, via
    /// [`store_path_spellings`], to build the path it expects to see refused,
    /// and again inside [`global_memory_gate`] when the assertion runs. Both go
    /// through `biorouter_mcp::global_memory_dir()` -> `config_dir()`, which
    /// reads `BIOROUTER_PATH_ROOT` each time.
    ///
    /// Other tests in this binary legitimately point that variable at their own
    /// temporary root and put it back — `logging.rs`, `managed/mod.rs`,
    /// `providers/utils.rs` and `session/diagnostics.rs` all do, correctly,
    /// under `env_lock`. This module took no lock at all, so one of those could
    /// land between our two resolutions: the expected path was built against
    /// one root and the gate matched against another, no refusal fired, and the
    /// test failed claiming
    ///
    /// ```text
    /// developer__shell {"command":"rm -rf …/config/memory"}
    ///   points at the machine-wide store and must be refused
    /// ```
    ///
    /// which reads as a hole in the #63 consent gate and is really a harness
    /// race. Measured at 6 failures in 20 full-suite runs before this guard.
    ///
    /// The writers were never the problem and adding a lock to them would not
    /// help — `env_lock` serialises only the tasks that ASK for it, and the
    /// reader here never did.
    ///
    /// ⚠ It pins the **sandbox** root rather than the variable's current value,
    /// and the version that pinned "current" was this same bug one level down:
    /// the read that decides what to pin happens *before* the lock is acquired,
    /// so when a relocating test holds the lock at that moment it is that
    /// test's `TempDir` that gets pinned — restored and deleted a moment later,
    /// while this test spends its whole life resolving under it. Holding the
    /// lock was always the point; reading the environment to decide what to
    /// hold was the hole. See `crate::test_sandbox::pin_sandbox_path_root`.
    fn pinned_store_root() -> env_lock::EnvGuard<'static> {
        crate::test_sandbox::pin_sandbox_path_root()
    }

    fn store_path_spellings() -> Vec<String> {
        let store = biorouter_mcp::global_memory_dir();
        let mut forms = vec![store.to_string_lossy().into_owned()];
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            if let Ok(tail) = store.strip_prefix(std::path::Path::new(&home)) {
                let tail = tail.to_string_lossy();
                forms.push(format!("~/{tail}"));
                forms.push(format!("$HOME/{tail}"));
                forms.push(format!("${{HOME}}/{tail}"));
            }
        }
        forms
    }

    /// #63 review, finding 2. The gate matches four tool *names*; the store is a
    /// directory of text files. `cat <store>/clinical.txt` is the same
    /// disclosure the consent card exists for, and `rm -rf <store>` is the same
    /// destruction, taken with a tool the gate does not recognise.
    ///
    /// The developer and computer-controller servers refuse the store at the
    /// filesystem boundary, which is unforgeable — but the shell resolves no
    /// path, so its literal references have to be caught here. Denied, not
    /// asked: an approval card for "run this shell command" is not consent to
    /// disclose a memory category, and there is a call that *is*.
    #[test]
    fn a_tool_that_names_the_store_path_is_refused() {
        let _root = pinned_store_root();
        for store in store_path_spellings() {
            for (tool, arguments) in [
                (
                    "developer__shell",
                    json!({"command": format!("cat {store}/clinical.txt")}),
                ),
                (
                    "developer__shell",
                    json!({"command": format!("rm -rf {store}")}),
                ),
                (
                    "developer__text_editor",
                    json!({"command": "view", "path": format!("{store}/clinical.txt")}),
                ),
                (
                    "computercontroller__cache",
                    json!({"command": "delete", "path": format!("{store}/clinical.txt")}),
                ),
                // Nested inside a structure, and inside a script body.
                (
                    "some__tool",
                    json!({"opts": {"paths": [format!("{store}/clinical.txt")]}}),
                ),
                (
                    "code_execution__execute_code",
                    json!({"code": format!("shell({{command: 'cat {store}/clinical.txt'}})")}),
                ),
            ] {
                let gate = global_memory_gate(tool, &args(arguments.clone()));
                let Some(GlobalMemoryGate::Refuse(reason)) = gate else {
                    panic!("{tool} {arguments} reached the machine-wide store, got {gate:?}");
                };
                assert!(
                    reason.contains("retrieve_memories"),
                    "the refusal must name the call that does work: {reason}"
                );
            }
        }
    }

    /// Denying by path must not deny by resemblance. A sibling of the store, the
    /// store's *parent*, and the project-local store are all ordinary paths.
    #[test]
    fn only_the_store_itself_is_refused_by_path() {
        let store = biorouter_mcp::global_memory_dir();
        let store = store.to_string_lossy();
        let parent = biorouter_mcp::global_memory_dir();
        let parent = parent.parent().unwrap().to_string_lossy().into_owned();

        for (tool, arguments) in [
            // A sibling whose name merely starts with the store's.
            (
                "developer__shell",
                json!({"command": format!("cat {store}-archive.txt")}),
            ),
            (
                "developer__shell",
                json!({"command": format!("cat {store}notes")}),
            ),
            // The config directory the store lives in — backing up ~/.config is
            // an ordinary thing to ask for.
            (
                "developer__shell",
                json!({"command": format!("tar czf backup.tgz {parent}")}),
            ),
            // Project-local memory: under the directory the user opened.
            (
                "developer__text_editor",
                json!({"command": "view", "path": ".biorouter/memory/development.txt"}),
            ),
            ("developer__shell", json!({"command": "ls -la /tmp"})),
        ] {
            assert!(
                global_memory_gate(tool, &args(arguments.clone())).is_none(),
                "{tool} {arguments} is not the machine-wide store and must not be refused"
            );
        }
    }

    /// #63 review, second round. Denying by path must not deny by *mention*.
    ///
    /// The first version of the check above walked every string in the payload,
    /// so any argument that contained the store's path anywhere was refused —
    /// and `names_path` ends a match on a quote, a backtick or whitespace, which
    /// is exactly how prose spells a path. The result was that documenting the
    /// memory feature became impossible: this repository's own
    /// `docs/extensions/built-in/memory.md` quotes the store's path in a table,
    /// so an agent could not edit it, could not write the release notes for this
    /// very feature, and could not commit either of them.
    ///
    /// Nothing here reaches the store. A `file_text` is written to the `path`
    /// beside it, and a commit message is not a file at all. The store's real
    /// protection is the storage boundary (`global_memory_file_barrier.rs`),
    /// which resolves paths and cannot be talked out of it.
    #[test]
    fn prose_that_merely_quotes_the_store_path_is_not_refused() {
        let _root = pinned_store_root();
        for store in store_path_spellings() {
            for (tool, arguments) in [
                // The regression as review found it: documentation about the
                // memory feature, written to an ordinary docs file.
                (
                    "developer__text_editor",
                    json!({
                        "command": "write",
                        "path": "docs/foo.md",
                        "file_text": format!(
                            "Global memories live in `{store}` and are shared by every session.\n"
                        ),
                    }),
                ),
                // The same path in a plain sentence, no backticks — `names_path`
                // ends a match on whitespace too.
                (
                    "developer__text_editor",
                    json!({
                        "command": "write",
                        "path": "docs/releases/notes/v1.88.6.md",
                        "file_text": format!(
                            "The consent gate protects {store}, the machine-wide store.\n"
                        ),
                    }),
                ),
                // An edit to an existing doc: the quoted path is in the text
                // being replaced and in its replacement.
                (
                    "developer__text_editor",
                    json!({
                        "command": "str_replace",
                        "path": "docs/extensions/built-in/memory.md",
                        "old_str": format!("| Global (user-wide) | `{store}/` |"),
                        "new_str": format!("| Global (machine-wide) | `{store}/` |"),
                    }),
                ),
                // Committing that documentation. The store's path is inside the
                // `-m` message: one shell word, and an English sentence rather
                // than a path operand.
                (
                    "developer__shell",
                    json!({"command": format!(
                        "git commit -m \"docs(memory): say that {store} is shared machine-wide\""
                    )}),
                ),
                (
                    "developer__shell",
                    json!({"command": format!(
                        "git commit -m 'docs: the global store lives at {store} on macOS'"
                    )}),
                ),
                // Telling the *user* where their memories are is the whole job
                // of the consent card, so the agent must be able to say it.
                (
                    "developer__text_editor",
                    json!({
                        "command": "write",
                        "path": "README.md",
                        "file_text": format!("Your global memories are in {store}. Delete them\n\
                                              from Settings → Chat → Memory.\n"),
                    }),
                ),
            ] {
                assert!(
                    global_memory_gate(tool, &args(arguments.clone())).is_none(),
                    "{tool} {arguments} only *mentions* the store and must not be refused"
                );
            }
        }
    }

    /// The narrowing above must not become a bypass: an argument that is a path,
    /// or a shell word in a path position, still names the store operatively and
    /// is still refused. This is the discriminating half of the test above — a
    /// fix that let these through would be worse than the regression.
    #[test]
    fn a_path_argument_naming_the_store_is_still_refused_after_the_narrowing() {
        let _root = pinned_store_root();
        for store in store_path_spellings() {
            for (tool, arguments) in [
                (
                    "developer__text_editor",
                    json!({"command": "view", "path": format!("{store}/clinical.txt")}),
                ),
                // The same call whose *content* is innocent: the target is the
                // store, which is all that matters.
                (
                    "developer__text_editor",
                    json!({
                        "command": "write",
                        "path": format!("{store}/planted.txt"),
                        "file_text": "an ordinary sentence\n",
                    }),
                ),
                (
                    "developer__shell",
                    json!({"command": format!("cat {store}/clinical.txt")}),
                ),
                (
                    "developer__shell",
                    json!({"command": format!("rm -rf {store}")}),
                ),
                // Quoted, because a path with a space would have to be.
                (
                    "developer__shell",
                    json!({"command": format!("cat \"{store}/clinical.txt\"")}),
                ),
            ] {
                assert!(
                    matches!(
                        global_memory_gate(tool, &args(arguments.clone())),
                        Some(GlobalMemoryGate::Refuse(_))
                    ),
                    "{tool} {arguments} points at the machine-wide store and must be refused"
                );
            }
        }
    }

    /// The check is now keyed on *which argument* carries the path, so it is
    /// worth stating what the key set is for — and that a documentation file
    /// this repository actually ships passes it.
    ///
    /// Constructed as a real tool call against the real file contents rather
    /// than argued about: `docs/extensions/built-in/memory.md` quotes the
    /// store's path in its storage table, and rewriting it is an ordinary edit.
    #[test]
    fn this_repositorys_own_documentation_can_still_be_edited() {
        let _root = pinned_store_root();
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("crates/biorouter has a workspace root above it")
            .to_path_buf();

        // The DOCUMENTED spelling, not the resolved one. `store_path_spellings`
        // asks `global_memory_dir()` where the store is right now, and this
        // binary answers with a temp directory: `crate::test_sandbox` sets
        // `BIOROUTER_PATH_ROOT` before any test runs, so a stray test cannot
        // write into the developer's real `~/.config/biorouter`. Prose in the
        // repository names the user-facing path and always will, so a runtime
        // form can never appear in it — the sampled docs would all miss, and the
        // vacuity assertion below would fire on a tree where nothing is wrong.
        // Every other test in this module wants the resolved path; this one
        // wants the one a human would type.
        let mut forms = vec!["~/.config/biorouter/memory".to_string()];
        forms.extend(store_path_spellings());

        let mut checked_one_that_quotes_the_store = false;
        for doc in [
            "CLAUDE.md",
            "docs/security/permission-modes.md",
            "docs/extensions/built-in/memory.md",
            "docs/agent-loop/designs/cross-session-memory.md",
        ] {
            let path = repo.join(doc);
            let Ok(body) = std::fs::read_to_string(&path) else {
                // A worktree may not carry every doc; the point is the ones it
                // does carry.
                continue;
            };
            if forms.iter().any(|form| names_path(&body, form)) {
                checked_one_that_quotes_the_store = true;
            }
            let gate = global_memory_gate(
                "developer__text_editor",
                &args(json!({
                    "command": "write",
                    "path": doc,
                    "file_text": body,
                })),
            );
            assert!(
                gate.is_none(),
                "an agent can no longer rewrite {doc}: {gate:?}"
            );
        }
        assert!(
            checked_one_that_quotes_the_store,
            "none of the sampled docs quote the store path, so this test proved \
             nothing; re-point it at a doc that does"
        );
    }

    // --- the uninspected dispatch boundaries ------------------------------

    /// #63 review, finding 3. The scan above reads *source text*, so it can
    /// always be out-computed — the review's one-liner is `const flag = true;
    /// retrieve_memories({category:"clinical", is_global:flag})`, which has no
    /// truthy literal after `is_global`. The boundary check reads the
    /// *dispatched call*, where `flag` has already become `true`, so the shape
    /// of the source is irrelevant to it.
    #[test]
    fn a_boundary_refusal_does_not_depend_on_how_the_call_was_written() {
        for boundary in [
            UninspectedBoundary::ExecuteCodeScript,
            UninspectedBoundary::AgentCallToolRoute,
        ] {
            for (tool, arguments) in [
                (
                    "memory__retrieve_memories",
                    json!({"category": "clinical", "is_global": true}),
                ),
                (
                    "memory__retrieve_memories",
                    json!({"category": "*", "is_global": true}),
                ),
                (
                    "memory__remember_memory",
                    json!({"category": "clinical", "data": "x", "is_global": true}),
                ),
                (
                    "memory__remove_memory_category",
                    json!({"category": "clinical", "is_global": true}),
                ),
                (
                    "memory__remove_memory_category",
                    json!({"category": "*", "is_global": true}),
                ),
                (
                    "memory__remove_specific_memory",
                    json!({"category": "clinical", "memory_content": "x", "is_global": true}),
                ),
                // Mounted under another name, and stringly-typed flag: the same
                // leniencies the gate itself applies.
                (
                    "personal_memory__retrieve_memories",
                    json!({"category": "clinical", "is_global": "true"}),
                ),
            ] {
                let refusal =
                    uninspected_boundary_refusal(tool, Some(&args(arguments.clone())), boundary);
                let message = refusal.unwrap_or_else(|| {
                    panic!("{boundary:?} let {tool} {arguments} through unasked")
                });
                assert!(
                    message.contains("machine-wide"),
                    "the refusal has to say what it protects: {message}"
                );
            }
        }
    }

    /// The boundary refuses machine-wide memory, not memory and not the
    /// boundary. Everything else keeps working, or the fix is an outage.
    #[test]
    fn a_boundary_leaves_local_memory_and_every_other_tool_alone() {
        for (tool, arguments) in [
            (
                "memory__retrieve_memories",
                json!({"category": "*", "is_global": false}),
            ),
            (
                "memory__remember_memory",
                json!({"category": "dev", "data": "x", "is_global": false}),
            ),
            (
                "memory__remove_memory_category",
                json!({"category": "*", "is_global": false}),
            ),
            // No flag at all is a local call, exactly as the gate reads it.
            ("memory__retrieve_memories", json!({"category": "dev"})),
            ("developer__shell", json!({"command": "ls"})),
            (
                "workflow__save_workflow",
                json!({"name": "x", "is_global": true}),
            ),
        ] {
            assert!(
                uninspected_boundary_refusal(
                    tool,
                    Some(&args(arguments.clone())),
                    UninspectedBoundary::ExecuteCodeScript
                )
                .is_none(),
                "{tool} {arguments} must pass the boundary"
            );
        }
        assert!(
            uninspected_boundary_refusal(
                "memory__retrieve_memories",
                None,
                UninspectedBoundary::ExecuteCodeScript
            )
            .is_none(),
            "a call with no arguments cannot be a global one"
        );
    }

    /// A boundary sees no inspector either, so finding 2's other shape has to be
    /// caught here as well: a script that skips the memory tools entirely and
    /// asks the shell to read the store's files.
    #[test]
    fn a_boundary_also_refuses_a_tool_that_names_the_store_path() {
        let store = biorouter_mcp::global_memory_dir();
        let store = store.to_string_lossy();
        assert!(
            uninspected_boundary_refusal(
                "developer__shell",
                Some(&args(
                    json!({"command": format!("cat {store}/clinical.txt")})
                )),
                UninspectedBoundary::ExecuteCodeScript,
            )
            .is_some(),
            "a script read the machine-wide store with the shell"
        );
        assert!(
            uninspected_boundary_refusal(
                "developer__shell",
                Some(&args(
                    json!({"command": format!("cat {store}-archive.txt")})
                )),
                UninspectedBoundary::ExecuteCodeScript,
            )
            .is_none(),
            "a sibling of the store is an ordinary file"
        );
    }

    /// A refusal that only says no is a dead end: the model retries, or tells
    /// the user the feature is broken. Each boundary names the route that does
    /// work.
    #[test]
    fn each_boundary_names_the_way_through() {
        let global = args(json!({"category": "clinical", "is_global": true}));

        let script = uninspected_boundary_refusal(
            "memory__retrieve_memories",
            Some(&global),
            UninspectedBoundary::ExecuteCodeScript,
        )
        .unwrap();
        assert!(
            script.contains("execute_code") && script.contains("retrieve_memories"),
            "the script refusal must say to make the call directly instead: {script}"
        );

        let route = uninspected_boundary_refusal(
            "memory__retrieve_memories",
            Some(&global),
            UninspectedBoundary::AgentCallToolRoute,
        )
        .unwrap();
        assert!(
            route.contains("conversation"),
            "the route refusal must point at the inspected path: {route}"
        );
    }

    #[test]
    fn execute_code_without_global_memory_is_not_escalated() {
        for code in [
            // Local memory work inside a script is ordinary.
            r#"import { retrieve_memories } from "memory";
retrieve_memories({ category: "*", is_global: false });"#,
            // No memory at all.
            r#"import { shell } from "developer";
shell({ command: "ls -la /tmp" });"#,
            // `is_global` belonging to something that is not a memory call.
            r#"import { save_workflow } from "workflow";
save_workflow({ name: "x", is_global: true });"#,
        ] {
            assert!(
                global_memory_gate("code_execution__execute_code", &args(json!({"code": code})))
                    .is_none(),
                "must not escalate:\n{code}"
            );
        }
    }

    // --- the inspector, end to end ----------------------------------------

    fn request(id: &str, name: &str, arguments: Value) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(CallToolRequestParams {
                task: None,
                name: name.to_string().into(),
                arguments: Some(args(arguments)),
                meta: None,
            }),
            metadata: None,
            tool_meta: None,
        }
    }

    /// Auto mode is the whole point: the permission inspector approves every
    /// tool there deterministically, so the gate has to fire anyway.
    #[tokio::test]
    async fn auto_mode_still_gates_a_global_read() {
        let results = GlobalMemoryInspector
            .inspect(
                &[request(
                    "r1",
                    "memory__retrieve_memories",
                    json!({"category": "clinical", "is_global": true}),
                )],
                &[],
                BioRouterMode::Auto,
                &crate::session::Session::default(),
            )
            .await
            .unwrap();
        let r = results
            .first()
            .expect("a global read must be gated in Auto");
        assert!(matches!(
            r.action,
            InspectionAction::RequireApproval(Some(_))
        ));
        assert_eq!(r.inspector_name, GLOBAL_MEMORY_INSPECTOR_NAME);
        assert!(r.finding_id.as_deref().unwrap_or("").starts_with("GMEM-"));
    }

    /// Unlike `sensitive_ops`, this gate is **not** inert outside Auto: a
    /// `SmartApprove` read-only grade would otherwise auto-approve the very same
    /// undisclosed cross-session read.
    #[tokio::test]
    async fn every_tool_running_mode_gates_a_global_read() {
        for mode in [
            BioRouterMode::Auto,
            BioRouterMode::Approve,
            BioRouterMode::SmartApprove,
        ] {
            let results = GlobalMemoryInspector
                .inspect(
                    &[request(
                        "r1",
                        "memory__retrieve_memories",
                        json!({"category": "clinical", "is_global": true}),
                    )],
                    &[],
                    mode,
                    &crate::session::Session::default(),
                )
                .await
                .unwrap();
            assert_eq!(results.len(), 1, "{mode:?} must gate the global read");
        }
    }

    #[tokio::test]
    async fn chat_mode_is_inert() {
        let results = GlobalMemoryInspector
            .inspect(
                &[request(
                    "r1",
                    "memory__retrieve_memories",
                    json!({"category": "clinical", "is_global": true}),
                )],
                &[],
                BioRouterMode::Chat,
                &crate::session::Session::default(),
            )
            .await
            .unwrap();
        assert!(results.is_empty(), "Chat dispatches no tools at all");
    }

    #[tokio::test]
    async fn the_whole_store_read_reaches_the_pipeline_as_a_deny() {
        let results = GlobalMemoryInspector
            .inspect(
                &[request(
                    "r1",
                    "memory__retrieve_memories",
                    json!({"category": "*", "is_global": true}),
                )],
                &[],
                BioRouterMode::Auto,
                &crate::session::Session::default(),
            )
            .await
            .unwrap();
        let r = results.first().expect("the bulk read must be gated");
        assert_eq!(r.action, InspectionAction::Deny);
        assert!(
            !r.reason.trim().is_empty(),
            "a refusal with no reason reaches the model as \"the user declined\", \
             which is both untrue and unactionable"
        );
    }
}
